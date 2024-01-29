use serde::{Deserialize, Serialize};

///User in OJ
#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Option<i32>,
    pub name: String,
}
