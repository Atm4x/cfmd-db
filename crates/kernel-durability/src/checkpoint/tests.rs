use kernel_model::{DatabaseState, Value};
use kernel_revision::Revision;
use kernel_schema::{
    CapabilityDef, FieldDef, RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext,
    SemanticEnvironment, StructuralEquivalenceDef, StructuralOrderingDef, Symbol, SymbolKind,
    TypeExpr, TypeVar,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{EntityId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};
use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn sid(raw: u128) -> SemanticId {
    SemanticId::new(raw)
}

fn complex_revision() -> (Revision, SemanticRegistry) {
    let entity_type = sid(1);
    let subtype = sid(2);
    let field = sid(3);
    let relation = sid(4);
    let capability = sid(5);
    let recursive_type = sid(6);
    let structural = sid(7);
    let structural_order = sid(8);
    let eq_i64 = sid(20);
    let eq_text = sid(21);
    let order_i64 = sid(22);

    let mut registry = SemanticRegistry::default();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9));
    environment.pin_module(
        eq_i64,
        registry.install_equivalence(EquivalenceModule::I64Exact),
    );
    environment.pin_module(
        eq_text,
        registry.install_equivalence(EquivalenceModule::TextExact),
    );
    environment.pin_module(
        order_i64,
        registry.install_ordering(OrderingModule::I64Ascending),
    );

    let mut schema = Schema::new(SchemaRevisionId::new(8));
    for (id, kind, name) in [
        (entity_type, SymbolKind::Entity, "entity"),
        (subtype, SymbolKind::Entity, "subtype"),
        (field, SymbolKind::Field, "name"),
        (relation, SymbolKind::Relation, "events"),
        (capability, SymbolKind::Capability, "readable"),
        (recursive_type, SymbolKind::Value, "recursive"),
    ] {
        schema
            .define(Symbol {
                id,
                kind,
                presentation_name: name.into(),
            })
            .unwrap();
    }
    schema
        .define_type(
            recursive_type,
            TypeExpr::Mu {
                binder: TypeVar(1),
                body: Box::new(TypeExpr::Seq(Box::new(TypeExpr::Var(TypeVar(1))))),
            },
        )
        .unwrap();
    schema
        .define_field(FieldDef {
            id: field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::Text),
        })
        .unwrap();
    schema
        .define_capability(CapabilityDef {
            id: capability,
            required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
        })
        .unwrap();
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::Text),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![eq_i64, eq_text],
            },
        })
        .unwrap();
    schema
        .define_structural_equivalence(
            structural,
            StructuralEquivalenceDef::Seq { element: eq_i64 },
        )
        .unwrap();
    schema
        .define_structural_ordering(
            structural_order,
            StructuralOrderingDef::Seq { element: order_i64 },
        )
        .unwrap();
    schema.include(capability, entity_type).unwrap();
    schema.include(subtype, entity_type).unwrap();

    let context = SemanticContext {
        schema,
        environment,
    };
    let state = complex_state(entity_type, field, relation);
    (
        Revision::build(RevisionId::new(44), &context, &registry, state).unwrap(),
        registry,
    )
}

fn complex_state(
    entity_type: SemanticId,
    field: SemanticId,
    relation: SemanticId,
) -> DatabaseState {
    let entity = EntityId::new(100);
    let mut state = DatabaseState::default();
    state.lifecycle.entities.insert(entity);
    state.lifecycle.roots.insert(entity);
    state
        .model
        .carriers
        .insert(entity_type, BTreeSet::from([entity]));
    state
        .model
        .fields
        .insert((field, entity), Value::Text("alpha".into()));
    state.model.relations.insert(
        relation,
        vec![vec![Value::I64(7), Value::Text("payload".into())]],
    );
    state
}

#[test]
fn full_revision_checkpoint_codec_roundtrips_schema_semantics_lifecycle_and_model() {
    let (revision, registry) = complex_revision();
    let bytes = encode_revision(&revision).unwrap();
    let decoded = decode_revision(&bytes, &registry).unwrap();
    assert_eq!(decoded, revision);
}
