pub mod apworlds;
pub mod edit;
pub mod gen;
pub mod list;
pub mod main;

pub use main::YamlContent;

pub fn routes() -> Vec<rocket::Route> {
    let mut routes = main::routes();
    routes.extend(apworlds::routes());
    routes.extend(edit::routes());
    routes.extend(gen::routes());
    routes.extend(list::routes());
    routes
}
