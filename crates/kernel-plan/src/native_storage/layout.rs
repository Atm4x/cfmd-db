#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutId(pub u128);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayoutFamily {
    LogicalModelRows,
    RowStore,
    Columnar,
    KeyValue,
    AdjacencyList,
    DenseArray,
    Inverted,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LayoutBinding {
    pub id: LayoutId,
    pub family: LayoutFamily,
}

impl LayoutBinding {
    pub const LOGICAL_MODEL_ROWS: Self = Self {
        id: LayoutId(0),
        family: LayoutFamily::LogicalModelRows,
    };

    /// Deterministic correctness-first physical representation used when a
    /// runtime root is rebuilt from semantic authority after recovery.
    pub const RECOVERY_ROW_STORE: Self = Self {
        id: LayoutId(1),
        family: LayoutFamily::RowStore,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct I64IndexBinding {
    pub relation: SemanticId,
    pub layout: LayoutBinding,
    pub key_column: usize,
    pub equivalence: SemanticId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticIndexKeyPart {
    pub column: usize,
    pub equivalence: SemanticId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticIndexBinding {
    pub relation: SemanticId,
    pub layout: LayoutBinding,
    pub key_parts: Vec<SemanticIndexKeyPart>,
}

impl SemanticIndexBinding {
    #[must_use]
    pub fn single(
        relation: SemanticId,
        layout: LayoutBinding,
        column: usize,
        equivalence: SemanticId,
    ) -> Self {
        Self {
            relation,
            layout,
            key_parts: vec![SemanticIndexKeyPart {
                column,
                equivalence,
            }],
        }
    }
}

