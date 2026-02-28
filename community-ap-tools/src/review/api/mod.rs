mod evaluate;
mod presets;
mod review;
mod teams;

pub fn routes() -> Vec<rocket::Route> {
    let mut routes = presets::routes();
    routes.extend(evaluate::routes());
    routes.extend(review::routes());
    routes
}

pub fn admin_routes() -> Vec<rocket::Route> {
    teams::routes()
}
