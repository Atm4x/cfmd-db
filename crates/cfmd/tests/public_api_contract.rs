//! Compile-time contract for the supported public Rust SDK boundary.
//!
//! Intentional breaking changes must update this file together with the productization ledger.

use std::path::Path;

use cfmd::dynamic::{Query, QueryWatch, RelationId, WatchEvent};
use cfmd::{
    Candidate, CandidateDerivedEffects, CfmdEntity, CfmdSchema, Database, DatabaseBuilder, Diagnostic, EntitySet, Error,
    ErrorDiagnosticExt, Id, Many, ManySelection, Object, ObjectQuery, Plan, PrincipalId,
    DatabaseContext, QueryNodeId, QuerySource, Ref, Result, Schema, Snapshot, Storage, Transaction,
    TransactionId,
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

fn typed_snapshot(context: &DatabaseContext<ContractSchema>) -> Result<Snapshot<ContractSchema>> {
    context.snapshot()
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

fn relationship_selection_api(
    selection: &ManySelection<ContractChild>,
    transaction: &mut Transaction,
) -> Result<(Vec<Id<ContractChild>>, usize)> {
    let ids = selection.ids()?;
    let count = selection.count()?;
    selection.detach_all(transaction)?;
    Ok((ids, count))
}

fn derived_preview_api(effects: CandidateDerivedEffects) -> (usize, usize) {
    (
        effects.normalized_rows_removed(),
        effects.orphan_entities_deleted(),
    )
}

fn transaction_api(database: &Database, plan: Plan) -> Result<()> {
    let mut transaction = Transaction::new();
    database.objects::<ContractItem>()?.add(
        &mut transaction,
        ContractItem {
            id: Id::new(1),
            name: "contract".to_owned(),
        },
    )?;
    let _ = transaction.id();
    let _ = transaction.origin_revision();
    transaction.add_plan(plan)?;
    let _ = database.preview(&transaction)?;
    let _ = database.commit(&transaction)?;
    Ok(())
}

fn public_types_exist(
    _storage: Storage,
    _schema: Schema,
    _plan: Option<Plan>,
    _candidate: Option<Candidate>,
    _transaction: TransactionId,
    _principal: PrincipalId,
    _reference: Ref<ContractItem>,
) {
}

#[test]
fn public_contract_compiles_as_documented() {
    let _ = database_builder as fn(&Path) -> DatabaseBuilder;
    let _ = typed_snapshot as fn(&DatabaseContext<ContractSchema>) -> Result<Snapshot<ContractSchema>>;
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
    let _ = relationship_selection_api
        as fn(
            &ManySelection<ContractChild>,
            &mut Transaction,
        ) -> Result<(Vec<Id<ContractChild>>, usize)>;
    let _ = derived_preview_api as fn(CandidateDerivedEffects) -> (usize, usize);
    let _ = transaction_api as fn(&Database, Plan) -> Result<()>;
    let _ = ContractSchema::database(Path::new("."));
    let _ = public_types_exist;
}
