use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use indexmap::IndexMap;


#[derive(Serialize, Deserialize, Debug)]
pub struct DataPackage {
    pub data: DataPackageData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DataPackageData {
    pub games: BTreeMap<String, HashedGameData>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct HashedGameData {
    pub checksum: String,
    #[serde(flatten)]
    pub game_data: GameData,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct GameData {
    #[serde(default)]
    pub item_name_groups: IndexMap<String, Vec<String>>,
    pub item_name_to_id: IndexMap<String, ItemId>,
    #[serde(default)]
    pub location_name_groups: IndexMap<String, Vec<String>>,
    pub location_name_to_id: IndexMap<String, LocationId>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct ItemId(pub i64);

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct LocationId(pub i64);
