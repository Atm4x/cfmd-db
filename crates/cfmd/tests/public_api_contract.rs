//! Compile-time contract for the supported public Rust SDK boundary.
//!
//! Intentional breaking changes must update this file together with the productization ledger.

use std::path::Path;

use cfmd::dynamic::{Query, QueryWatch, RelationId, WatchEvent};
use cfmd::{
    Candidate, CandidateDerivedEffects, CfmdEntity, Database, DatabaseBuilder, Diagnostic, Error,
    ErrorDiagnosticExt, Id, Many, ManySelection, Object, ObjectQuery, Plan, PrincipalId,
    QueryNodeId, QuerySource, ReadContext, Ref, Result, Schema, Storage, Transaction,
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

fn database_builder(path: &Path) -> DatabaseBuilder {
    Database::builder(path)
}

fn snapshot(database: &Database) -> Result<ReadContext> {
    database.snapshot()
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
) -> Result<(Vec<Id<ContractChild>>, usize, Plan)> {
    Ok((
        selection.ids()?,
        selection.count()?,
        selection.detach_all()?,
    ))
}

fn derived_preview_api(effects: CandidateDerivedEffects) -> (usize, usize) {
    (
        effects.normalized_rows_removed(),
        effects.orphan_entities_deleted(),
    )
}

fn transaction_api(transaction: &mut Transaction, plan: Plan) -> Result<()> {
    let _ = transaction.id();
    let _ = transaction.base_revision();
    let _ = transaction.read();
    transaction.apply(plan)?;
    let _ = transaction.preview()?;
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
    let _ = snapshot as fn(&Database) -> Result<ReadContext>;
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
        as fn(&ManySelection<ContractChild>) -> Result<(Vec<Id<ContractChild>>, usize, Plan)>;
    let _ = derived_preview_api as fn(CandidateDerivedEffects) -> (usize, usize);
    let _ = transaction_api as fn(&mut Transaction, Plan) -> Result<()>;
    let _ = public_types_exist;
}
