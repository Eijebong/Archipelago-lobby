use anyhow::bail;
use diesel::serialize::WriteTuple;
use diesel::sql_types::{Record, Text};
use diesel::{deserialize::FromSql, pg::Pg, query_builder::QueryId, serialize::ToSql};
use diesel::{AsExpression, FromSqlRow, SqlType};
use rocket::form::FromForm;
use rocket::request::FromParam;
use rocket::UriDisplayPath;
use semver::Version;
use std::fmt::Display;
use std::str::FromStr;
use uuid::Uuid;

use crate::schema::sql_types::{Apworld, ValidationStatus};

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
        use serde::{Deserialize, Serialize};
        $(
            #[derive(Clone, Copy, Debug, UriDisplayPath, FromForm, Deserialize, Serialize, PartialEq, Eq, Hash, Ord, PartialOrd)]
            #[serde(transparent)]
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

                pub fn as_generic_id(&self) -> uuid::Uuid {
                    self.0
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
    SqlRoomTemplateId => RoomTemplateId,
    SqlBundleId => BundleId,
);

#[derive(Debug, Clone, Copy, FromSqlRow, AsExpression, PartialEq, Serialize)]
#[diesel(sql_type=ValidationStatus)]
pub enum YamlValidationStatus {
    Validated,
    ManuallyValidated,
    Unsupported,
    Failed,
    Unknown,
}

impl YamlValidationStatus {
    pub fn as_str(&self) -> &str {
        match self {
            YamlValidationStatus::Validated => "validated",
            YamlValidationStatus::ManuallyValidated => "manually_validated",
            YamlValidationStatus::Unsupported => "unsupported",
            YamlValidationStatus::Failed => "failed",
            YamlValidationStatus::Unknown => "unknown",
        }
    }

    pub fn is_valid(&self) -> bool {
        matches!(
            self,
            YamlValidationStatus::Validated | YamlValidationStatus::ManuallyValidated
        )
    }
}

impl FromStr for YamlValidationStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "validated" => Self::Validated,
            "manually_validated" => Self::ManuallyValidated,
            "unsupported" => Self::Unsupported,
            "failed" => Self::Failed,
            "unknown" => Self::Unknown,
            other => bail!("Unknown variant for YamlValidationStatus: {}", other),
        })
    }
}

impl ToSql<ValidationStatus, Pg> for YamlValidationStatus {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Pg>,
    ) -> diesel::serialize::Result {
        <&str as ToSql<Text, Pg>>::to_sql(&self.as_str(), &mut out.reborrow())
    }
}

impl FromSql<ValidationStatus, Pg> for YamlValidationStatus {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        let value = <String as FromSql<Text, Pg>>::from_sql(bytes)?;

        Ok(YamlValidationStatus::from_str(&value)?)
    }
}

impl FromSql<Apworld, Pg> for (String, Version) {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        let (name, version): (String, String) =
            FromSql::<Record<(Text, Text)>, Pg>::from_sql(bytes)?;

        Ok((name, version.parse()?))
    }
}

impl ToSql<Apworld, Pg> for (String, Version) {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Pg>,
    ) -> diesel::serialize::Result {
        WriteTuple::<(Text, Text)>::write_tuple(&(&self.0, self.1.to_string()), &mut out.reborrow())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::db::YamlValidationStatus;

    #[test]
    fn test_roundtrip_validation_status() {
        fn test_one(status: YamlValidationStatus) {
            assert_eq!(
                status,
                YamlValidationStatus::from_str(status.as_str())
                    .expect("Failed to parse validation status")
            )
        }

        test_one(YamlValidationStatus::Validated);
        test_one(YamlValidationStatus::ManuallyValidated);
        test_one(YamlValidationStatus::Unsupported);
        test_one(YamlValidationStatus::Failed);
        test_one(YamlValidationStatus::Unknown);
    }
}
