use diesel::{deserialize::FromSql, pg::Pg, query_builder::QueryId, serialize::ToSql};
use diesel::{AsExpression, FromSqlRow, SqlType};
use rocket::request::FromParam;
use rocket::UriDisplayPath;
use std::fmt::Display;
use uuid::Uuid;

macro_rules! new_id_type {
    (@sql_types $($sql_type:ident),+) => {
        pub mod sql {
            use super::{SqlType, QueryId};
            $(
                #[derive(SqlType, QueryId)]
                #[diesel(postgres_type(name = "uuid"))]
                pub struct $sql_type;
            )+
        }
    };
    ($($sql_type:ident => $rust_type:ident,)+) => {
        new_id_type!(@sql_types $($sql_type),+);

        use sql::*;
        $(
            #[derive(Clone, Copy, Debug, UriDisplayPath)]
            #[derive(FromSqlRow, AsExpression)]
            #[diesel(sql_type=$sql_type)]
            pub struct $rust_type(Uuid);

            impl ToSql<$sql_type, Pg> for $rust_type {
                fn to_sql<'b>(&'b self, out: &mut diesel::serialize::Output<'b, '_, Pg>) -> diesel::serialize::Result {
                    ToSql::<diesel::sql_types::Uuid, Pg>::to_sql(&self.0, out)
                }
            }

            impl FromSql<$sql_type, Pg> for $rust_type {
                fn from_sql(bytes: <Pg as diesel::backend::Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
                    Ok($rust_type(Uuid::from_sql(bytes)?))
                }
            }

            impl $rust_type {
                pub fn new_v4() -> Self {
                    Self(Uuid::new_v4())
                }
            }

            impl<'a> FromParam<'a> for $rust_type {
                type Error = <Uuid as FromParam<'a>>::Error;

                fn from_param(param: &'a str) -> std::result::Result<Self, Self::Error> {
                    Ok(Self(Uuid::from_param(param)?))
                }
            }

            impl Display for $rust_type {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    self.0.fmt(f)
                }
            }
        )+
    };
}

new_id_type!(
    SqlRoomId => RoomId,
    SqlYamlId => YamlId,
);
