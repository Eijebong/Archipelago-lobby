use ap_lobby::db::{self, RoomFilter};
use ap_lobby::error::Result;
use diesel_async::{
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager},
    AsyncPgConnection,
};
use dotenvy::dotenv;
use http::{HeaderName, HeaderValue};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }

    let db_url = std::env::var("DATABASE_URL").expect("Plox provide a DATABASE_URL env variable");
    let lobby_root =
        std::env::var("LOBBY_ROOT_URL").expect("Plox provide a LOBBY_ROOT_URL env variable");
    let lobby_api_key =
        std::env::var("LOBBY_API_KEY").expect("Plox provide a LOBBY_API_KEY env variable");
    let mgr = AsyncDieselConnectionManager::<AsyncPgConnection>::new(db_url);
    let db_pool = Pool::builder(mgr)
        .build()
        .expect("Failed to create database pool, aborting");

    let mut conn = db_pool.get().await?;

    let all_rooms = RoomFilter::default().with_open_state(ap_lobby::db::OpenState::Open);

    let (rooms, _) = db::list_rooms(all_rooms, None, &mut conn).await?;
    let client = reqwest::Client::new();

    for room in rooms {
        if !room.settings.yaml_validation {
            continue;
        }

        let yamls = db::get_yamls_for_room(room.id, &mut conn).await?;
        for yaml in yamls {
            let url = format!("{}/api/room/{}/retry/{}", lobby_root, room.id, yaml.id);
            dbg!(&url);
            client
                .get(&url)
                .header(
                    HeaderName::from_static("x-api-key"),
                    HeaderValue::from_str(&lobby_api_key)?,
                )
                .send()
                .await?;
        }
    }

    Ok(())
}
