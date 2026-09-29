use cfmd::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.todo")]
struct Todo {
    #[cfmd(id)]
    pub id: Id<Todo>,
    pub title: String,
    pub done: bool,
}

fn main() -> Result<()> {
    let path = std::env::temp_dir().join("cfmd-todo-example.cfmd");
    let _ = std::fs::remove_file(&path);
    let schema = Schema::builder().object::<Todo>().build()?;
    let db = Database::builder(&path).schema(schema).create()?;

    let snapshot = db.snapshot()?;
    let todos = snapshot.objects::<Todo>()?;
    let plan = todos.insert(Todo {
        id: Id::new(1),
        title: "Try CFMD".to_owned(),
        done: false,
    })?;
    drop(todos);
    drop(snapshot);
    db.commit(&plan, TransactionId::new(1))?;

    let snapshot = db.snapshot()?;
    let todo = snapshot.objects::<Todo>()?.require(Id::new(1))?;
    println!("{}: done={}", todo.title, todo.done);
    drop(snapshot);
    drop(db);
    let _ = std::fs::remove_file(path);
    Ok(())
}
