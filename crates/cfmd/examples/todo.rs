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
    let db = Database::builder(&path)
        .schema(TodoSchema::definition()?)
        .create()?;

    let context = db.context::<TodoSchema>()?;
    context.add(
        |schema| &schema.todos,
        Todo {
            id: Id::new(1),
            title: "Try CFMD".to_owned(),
            done: false,
        },
    )?;
    context.commit()?;

    let read = db.context::<TodoSchema>()?;
    let todo = read.todos.require(Id::new(1))?;
    println!("{}: done={}", todo.title, todo.done);
    drop(read);
    drop(context);
    drop(db);
    let _ = std::fs::remove_file(path);
    Ok(())
}
