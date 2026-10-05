//! Compile-time contract for the supported public Rust SDK boundary.
//!
//! Intentional breaking changes must update this file together with the productization ledger.

use std::path::Path;

use cfmd::dynamic::{Query, QueryWatch, RelationId, WatchEvent};
use cfmd::{
    Candidate, CandidateDerivedEffects, CfmdEntity, CfmdSchema, Context, ContextAdmission,
    Database, DatabaseBuilder, Diagnostic, EntitySet, Error, ErrorDiagnosticExt, Id, Many, Object,
    ObjectQuery, Plan, PrincipalId, QueryNodeId, QuerySource, Ref, Result, Schema, Snapshot,
    Storage,
};

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "contract.item")]
struct ContractItem {
    #[cfmd(id)]
    id: Id<ContractItem>,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "contract.parent")]
struct ContractParent {
    #[cfmd(id)]
    id: Id<ContractParent>,
    name: String,
    children: Many<ContractChild>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "contract.child")]
struct ContractChild {
    #[cfmd(id)]
    id: Id<ContractChild>,
}

#[allow(dead_code)]
#[derive(CfmdSchema)]
struct ContractSchema {
    items: EntitySet<ContractItem>,
    parents: EntitySet<ContractParent>,
    children: EntitySet<ContractChild>,
}

fn database_builder(path: &Path) -> DatabaseBuilder {
    Database::builder(path)
}

fn typed_snapshot(context: &Context<ContractSchema>) -> Result<Snapshot<ContractSchema>> {
    context.snapshot()
}

fn context_admission(database: &Database) -> Result<ContextAdmission> {
    database.begin_context()
}

fn diagnostic(error: &Error) -> Diagnostic {
    error.diagnostic()
}

fn query_identity(query: &Query) -> (QueryNodeId, QuerySource) {
    (query.node_id(), query.source())
}

fn dynamic_watch_api(watch: &mut QueryWatch) -> Result<Option<WatchEvent>> {
    watch.try_recv()
}

fn object_query_identity(query: &ObjectQuery<ContractItem>) -> (QueryNodeId, QuerySource) {
    (query.node_id(), query.source())
}

fn relationship_api(
    reference: &Ref<ContractItem>,
    many: &Many<ContractChild>,
) -> Result<(ObjectQuery<ContractItem>, ObjectQuery<ContractChild>)> {
    Ok((reference.query()?, many.query()?))
}

fn derived_preview_api(effects: CandidateDerivedEffects) -> (usize, usize) {
    (
        effects.normalized_rows_removed(),
        effects.orphan_entities_deleted(),
    )
}

fn scoped_context_api(database: &Database) -> Result<()> {
    let context = database.context::<ContractSchema>()?;
    context.add(
        |schema| &schema.items,
        ContractItem {
            id: Id::new(1),
            name: "contract".to_owned(),
        },
    )?;
    let _ = context.preview()?;
    let _ = context.commit()?;
    Ok(())
}
fn public_types_exist(
    _storage: Storage,
    _schema: Schema,
    _plan: Option<Plan>,
    _candidate: Option<Candidate>,
    _principal: PrincipalId,
    _reference: Ref<ContractItem>,
) {
}

#[test]
fn public_contract_compiles_as_documented() {
    let _ = database_builder as fn(&Path) -> DatabaseBuilder;
    let _ = typed_snapshot as fn(&Context<ContractSchema>) -> Result<Snapshot<ContractSchema>>;
    let _ = context_admission as fn(&Database) -> Result<ContextAdmission>;
    let _ = diagnostic as fn(&Error) -> Diagnostic;
    let query = Query::scan(RelationId::new(1));
    let _ = query_identity(&query);
    let _ = dynamic_watch_api as fn(&mut QueryWatch) -> Result<Option<WatchEvent>>;
    let _ = object_query_identity as fn(&ObjectQuery<ContractItem>) -> (QueryNodeId, QuerySource);
    let _ = relationship_api
        as fn(
            &Ref<ContractItem>,
            &Many<ContractChild>,
        ) -> Result<(ObjectQuery<ContractItem>, ObjectQuery<ContractChild>)>;
    let _ = ContractParent::cfmd_new
        as fn(Id<ContractParent>, String, Many<ContractChild>) -> ContractParent;
    assert_eq!(ContractParent::fields().len(), 2);
    assert_eq!(ContractParent::many_fields().len(), 1);
    let _ = derived_preview_api as fn(CandidateDerivedEffects) -> (usize, usize);
    let _ = scoped_context_api as fn(&Database) -> Result<()>;
    let _ = public_types_exist;
}
