use cfmd::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.todo", authoritative)]
struct Todo {
    #[cfmd(id)]
    pub id: Id<Todo>,
    pub title: String,
    pub done: bool,
}

#[derive(CfmdSchema)]
struct TodoSchema {
    todos: EntitySet<Todo>,
}

fn main() -> Result<()> {
    let path = std::env::temp_dir().join("cfmd-todo-example.cfmd");
    let _ = std::fs::remove_file(&path);
    let db = TodoSchema::database(&path).create()?;

    let mut transaction = Transaction::new();
    db.todos.add(
        &mut transaction,
        Todo {
            id: Id::new(1),
            title: "Try CFMD".to_owned(),
            done: false,
        },
    )?;
    db.commit(&transaction)?;

    let todo = db.todos.require(Id::new(1))?;
    println!("{}: done={}", todo.title, todo.done);
    drop(transaction);
    drop(db);
    let _ = std::fs::remove_file(path);
    Ok(())
}
