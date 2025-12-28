use crate::db::{self, Author, Room, RoomFilter};
use crate::error::Result;
use crate::session::LoggedInSession;
use askama::Template;
use askama_web::WebTemplate;
use rocket::get;
use rocket::State;

use crate::{Context, TplContext};

#[derive(Template, WebTemplate)]
#[template(path = "room/list.html")]
struct ListRoomsTpl<'a> {
    base: TplContext<'a>,
    rooms: Vec<Room>,
    current_page: u64,
    max_pages: u64,
}

#[get("/rooms?<page>")]
#[tracing::instrument(skip_all)]
async fn my_rooms<'a>(
    ctx: &State<Context>,
    session: LoggedInSession,
    page: Option<u64>,
) -> Result<ListRoomsTpl<'a>> {
    let author_filter = if session.0.is_admin {
        Author::Any
    } else {
        Author::User(session.user_id())
    };

    let mut conn = ctx.db_pool.get().await?;
    let current_page = page.unwrap_or(1);

    let (rooms, max_pages) = db::list_rooms(
        RoomFilter::default().with_author(author_filter),
        Some(current_page),
        &mut conn,
    )
    .await?;

    Ok(ListRoomsTpl {
        base: TplContext::from_session("rooms", session.0, ctx).await,
        rooms,
        current_page,
        max_pages,
    })
}

pub fn routes() -> Vec<rocket::Route> {
    rocket::routes![my_rooms]
}
