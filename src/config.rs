use serde::{Deserialize, Serialize};

///Type of the problem's judging method in config.json
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemType {
    Standard,
    Strict,
    Spj,
    DynamicRanking,
}

///Case in one problem containing important information for judging
#[derive(Clone, Serialize, Deserialize)]
pub struct Case {
    pub score: f64,
    pub input_file: String,
    pub answer_file: String,
    pub time_limit: i32,
    pub memory_limit: i32,
}

///Advanced judging methods
#[derive(Clone, Serialize, Deserialize)]
pub struct Misc {
    pub packing: Option<Vec<Vec<i32>>>,
    pub special_judge: Option<Vec<String>>,
    pub dynamic_ranking_ratio: Option<f64>,
}

///One problem assigned in config.json containing its relevant information
#[derive(Clone, Serialize, Deserialize)]
pub struct Problem {
    pub id: i32,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ProblemType,
    pub misc: Option<Misc>,
    pub cases: Vec<Case>,
}

///Programming language and its argument commands for compiling
#[derive(Clone, Serialize, Deserialize)]
pub struct Language {
    pub name: String,
    pub file_name: String,
    pub command: Vec<String>,
}

///One server with its address and port
#[derive(Clone, Serialize, Deserialize)]
pub struct Server {
    pub bind_address: Option<String>,
    pub bind_port: Option<i32>,
}

///The whole config.json struct
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: Server,
    pub problems: Vec<Problem>,
    pub languages: Vec<Language>,
}
