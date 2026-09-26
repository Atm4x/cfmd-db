#[derive(Debug, Clone, PartialEq, Eq)]
struct PhysicalRowSlot {
    generation: u64,
    position: Option<usize>,
    previous: Option<PhysicalRowId>,
    next: Option<PhysicalRowId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledRelation {
    data: NativeRelation,
    row_ids: PersistentPhysicalVec<PhysicalRowId>,
    slots: PersistentPhysicalVec<PhysicalRowSlot>,
    free_slots: PersistentPhysicalVec<usize>,
    logical_head: Option<PhysicalRowId>,
    logical_tail: Option<PhysicalRowId>,
    scan_order_is_physical: bool,
}

fn installed_relation_estimated_retained_bytes(relation: &InstalledRelation) -> usize {
    std::mem::size_of::<InstalledRelation>()
        .saturating_add(native_relation_estimated_heap_bytes(&relation.data))
        .saturating_add(relation.row_ids.estimated_heap_bytes())
        .saturating_add(relation.slots.estimated_heap_bytes())
        .saturating_add(relation.free_slots.estimated_heap_bytes())
}

impl InstalledRelation {
    fn new(data: NativeRelation) -> Self {
        let row_count = native_relation::native_row_count(&data);
        let mut row_ids = Vec::with_capacity(row_count);
        let mut slots = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let id = PhysicalRowId {
                slot: index,
                generation: 0,
            };
            row_ids.push(id);
            slots.push(PhysicalRowSlot {
                generation: 0,
                position: Some(index),
                previous: index.checked_sub(1).map(|slot| PhysicalRowId {
                    slot,
                    generation: 0,
                }),
                next: (index + 1 < row_count).then_some(PhysicalRowId {
                    slot: index + 1,
                    generation: 0,
                }),
            });
        }
        Self {
            data,
            row_ids: PersistentPhysicalVec::from_vec(row_ids),
            slots: PersistentPhysicalVec::from_vec(slots),
            free_slots: PersistentPhysicalVec::default(),
            logical_head: (row_count > 0).then_some(PhysicalRowId {
                slot: 0,
                generation: 0,
            }),
            logical_tail: row_count.checked_sub(1).map(|slot| PhysicalRowId {
                slot,
                generation: 0,
            }),
            scan_order_is_physical: true,
        }
    }

    fn row_id_at(&self, index: usize) -> Result<PhysicalRowId, PhysicalExecutionError> {
        self.row_ids
            .get(index)
            .copied()
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)
    }

    fn position(&self, id: PhysicalRowId) -> Option<usize> {
        let slot = self.slots.get(id.slot)?;
        (slot.generation == id.generation)
            .then_some(slot.position)
            .flatten()
    }

    fn remove_row(&mut self, index: usize) -> Result<PhysicalRowId, PhysicalExecutionError> {
        let id = self.row_id_at(index)?;
        let next_generation = self
            .slots
            .get(id.slot)
            .filter(|slot| slot.generation == id.generation && slot.position.is_some())
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?
            .generation
            .checked_add(1)
            .ok_or(PhysicalExecutionError::HandleGenerationExhausted)?;
        native_relation::remove_native_row(&mut self.data, index)?;
        self.row_ids.swap_remove(index);
        self.scan_order_is_physical = false;
        if let Some(moved_id) = self.row_ids.get(index).copied() {
            self.slots[moved_id.slot].position = Some(index);
        }

        let removed_slot = self
            .slots
            .get(id.slot)
            .cloned()
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
        if removed_slot.generation != id.generation || removed_slot.position.is_none() {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        if let Some(previous) = removed_slot.previous {
            self.slots[previous.slot].next = removed_slot.next;
        } else {
            self.logical_head = removed_slot.next;
        }
        if let Some(next) = removed_slot.next {
            self.slots[next.slot].previous = removed_slot.previous;
        } else {
            self.logical_tail = removed_slot.previous;
        }
        let slot = &mut self.slots[id.slot];
        slot.position = None;
        slot.previous = None;
        slot.next = None;
        slot.generation = next_generation;
        self.free_slots.push(id.slot);
        Ok(id)
    }

    fn removal_rebuilds_data(&self) -> bool {
        let NativeRelation::TypedColumnar { columns, .. } = &self.data else {
            return false;
        };
        columns.iter().any(|column| {
            matches!(
                column,
                NativeColumn::Algebraic(column) if column.swap_remove_requires_rebuild()
            )
        })
    }

    fn remove_rows(&mut self, removed: &[PhysicalRowId]) -> Result<(), PhysicalExecutionError> {
        if removed.len() > 1 && self.removal_rebuilds_data() {
            return self.remove_rows_rebuild_once(removed);
        }
        for &row_id in removed {
            let position = self
                .position(row_id)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if self.remove_row(position)? != row_id {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        Ok(())
    }

    fn select_survivors(
        &self,
        positions: &[usize],
    ) -> Result<NativeRelation, PhysicalExecutionError> {
        match &self.data {
            NativeRelation::RowStore(rows) => Ok(NativeRelation::RowStore(
                select_persistent_positions(rows, positions)?,
            )),
            NativeRelation::Columnar { columns, .. } => Ok(NativeRelation::Columnar {
                columns: columns
                    .iter()
                    .map(|column| select_persistent_positions(column, positions))
                    .collect::<Result<Vec<_>, _>>()?,
                row_count: positions.len(),
            }),
            NativeRelation::I64Columnar { columns, .. } => Ok(NativeRelation::I64Columnar {
                columns: columns
                    .iter()
                    .map(|column| select_persistent_positions(column, positions))
                    .collect::<Result<Vec<_>, _>>()?,
                row_count: positions.len(),
            }),
            NativeRelation::TypedColumnar { columns, .. } => Ok(NativeRelation::TypedColumnar {
                columns: columns
                    .iter()
                    .map(|column| column.select_positions(positions))
                    .collect::<Result<Vec<_>, _>>()?,
                row_count: positions.len(),
            }),
        }
    }

    // HOSTILE[P193][ACTIVE][CLEAN]: rebuild-based algebraic carriers batch physical removals
    // into one survivor projection instead of replaying a full column rebuild per removed row.
    fn remove_rows_rebuild_once(
        &mut self,
        removed: &[PhysicalRowId],
    ) -> Result<(), PhysicalExecutionError> {
        if removed.is_empty() {
            return Ok(());
        }

        let mut physical_ids = (0..self.row_ids.len())
            .map(|position| self.row_id_at(position))
            .collect::<Result<Vec<_>, _>>()?;
        let mut source_positions = (0..physical_ids.len()).collect::<Vec<_>>();
        let mut positions_by_slot = vec![None; self.slots.len()];
        for (position, id) in physical_ids.iter().copied().enumerate() {
            positions_by_slot[id.slot] = Some((id.generation, position));
        }

        for &id in removed {
            let Some(Some((generation, position))) = positions_by_slot.get(id.slot).copied() else {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            };
            if generation != id.generation {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            if self
                .slots
                .get(id.slot)
                .is_some_and(|slot| slot.generation == u64::MAX)
            {
                return Err(PhysicalExecutionError::HandleGenerationExhausted);
            }

            physical_ids.swap_remove(position);
            source_positions.swap_remove(position);
            positions_by_slot[id.slot] = None;
            if let Some(&moved) = physical_ids.get(position) {
                positions_by_slot[moved.slot] = Some((moved.generation, position));
            }
        }

        let selected_data = self.select_survivors(&source_positions)?;
        let mut slots = self.slots.clone();
        let mut free_slots = self.free_slots.clone();
        let mut logical_head = self.logical_head;
        let mut logical_tail = self.logical_tail;

        for &id in removed {
            let removed_slot = slots
                .get(id.slot)
                .cloned()
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
            if removed_slot.generation != id.generation || removed_slot.position.is_none() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            if let Some(previous) = removed_slot.previous {
                slots[previous.slot].next = removed_slot.next;
            } else {
                logical_head = removed_slot.next;
            }
            if let Some(next) = removed_slot.next {
                slots[next.slot].previous = removed_slot.previous;
            } else {
                logical_tail = removed_slot.previous;
            }
            let slot = &mut slots[id.slot];
            slot.position = None;
            slot.previous = None;
            slot.next = None;
            slot.generation = slot
                .generation
                .checked_add(1)
                .ok_or(PhysicalExecutionError::HandleGenerationExhausted)?;
            free_slots.push(id.slot);
        }

        for (position, id) in physical_ids.iter().copied().enumerate() {
            slots[id.slot].position = Some(position);
        }

        self.data = selected_data;
        self.row_ids = PersistentPhysicalVec::from_vec(physical_ids);
        self.slots = slots;
        self.free_slots = free_slots;
        self.logical_head = logical_head;
        self.logical_tail = logical_tail;
        self.scan_order_is_physical = false;
        Ok(())
    }

    fn next_logical_id(&self, id: PhysicalRowId) -> Option<PhysicalRowId> {
        let slot = self.slots.get(id.slot)?;
        (slot.generation == id.generation)
            .then_some(slot.next)
            .flatten()
    }

    fn scan_positions(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if self.scan_order_is_physical {
            Box::new(0..self.row_ids.len())
        } else {
            Box::new(
                std::iter::successors(self.logical_head, |id| self.next_logical_id(*id))
                    .filter_map(|id| self.position(id)),
            )
        }
    }

    fn planned_insert_ids(
        &self,
        removed: &[PhysicalRowId],
        count: usize,
    ) -> Result<Vec<PhysicalRowId>, PhysicalExecutionError> {
        let mut fresh_slot = self.slots.len();
        (0..count)
            .map(|offset| {
                if offset < removed.len() {
                    let removed_id = removed[removed.len() - 1 - offset];
                    let generation = self.slots[removed_id.slot]
                        .generation
                        .checked_add(1)
                        .ok_or(PhysicalExecutionError::HandleGenerationExhausted)?;
                    Ok(PhysicalRowId {
                        slot: removed_id.slot,
                        generation,
                    })
                } else {
                    let free_offset = offset - removed.len();
                    if free_offset < self.free_slots.len() {
                        let slot = self.free_slots[self.free_slots.len() - 1 - free_offset];
                        Ok(PhysicalRowId {
                            slot,
                            generation: self.slots[slot].generation,
                        })
                    } else {
                        let id = PhysicalRowId {
                            slot: fresh_slot,
                            generation: 0,
                        };
                        fresh_slot += 1;
                        Ok(id)
                    }
                }
            })
            .collect()
    }

    fn push_row(
        &mut self,
        row: &kernel_query::Row,
    ) -> Result<PhysicalRowId, PhysicalExecutionError> {
        native_relation::push_native_row(&mut self.data, row)?;
        let position = self.row_ids.len();
        let id = if let Some(slot_index) = self.free_slots.pop() {
            let slot = &mut self.slots[slot_index];
            let id = PhysicalRowId {
                slot: slot_index,
                generation: slot.generation,
            };
            slot.position = Some(position);
            slot.previous = self.logical_tail;
            slot.next = None;
            id
        } else {
            let id = PhysicalRowId {
                slot: self.slots.len(),
                generation: 0,
            };
            self.slots.push(PhysicalRowSlot {
                generation: 0,
                position: Some(position),
                previous: self.logical_tail,
                next: None,
            });
            id
        };
        if let Some(previous) = self.logical_tail {
            self.slots[previous.slot].next = Some(id);
        } else {
            self.logical_head = Some(id);
        }
        self.logical_tail = Some(id);
        self.row_ids.push(id);
        Ok(id)
    }
}

