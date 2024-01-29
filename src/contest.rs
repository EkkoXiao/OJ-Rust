use crate::user::*;
use chrono::prelude::*;
use serde::{Deserialize, Serialize};

///Scoring rules including Latest and Highest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScoringRule {
    #[serde(rename = "latest")]
    Latest,
    #[serde(rename = "highest")]
    Highest,
}

///Used for breaking the tie
#[derive(Clone, Serialize, Deserialize)]
pub enum TieBreaker {
    #[serde(rename = "submission_time")]
    SubmissionTime,
    #[serde(rename = "submission_count")]
    SubmissionCount,
    #[serde(rename = "user_id")]
    UserId,
}

///When requesting for a contest the url needed, containing scoring rule and tie breaker
#[derive(Clone, Serialize, Deserialize)]
pub struct ContestRules {
    pub scoring_rule: Option<ScoringRule>,
    pub tie_breaker: Option<TieBreaker>,
}

///Users in one single contest
#[derive(Clone, Serialize, Deserialize)]
pub struct UserinContest {
    pub user: User,
    pub rank: i32,
    pub scores: Vec<f64>,
}

///User and further info in order to define his/her rank
#[derive(Clone)]
pub struct UserRanking {
    pub user_id: i32,
    pub user: User,
    pub scores: Vec<f64>,
    pub time: Vec<DateTime<Utc>>,
    pub count: i32,
    pub tot_score: f64,
    pub latest: DateTime<Utc>,
}

///Struct for one contest
#[derive(Clone, Serialize, Deserialize)]
pub struct Contest {
    pub id: Option<i32>,
    pub name: String,
    pub from: String,
    pub to: String,
    pub problem_ids: Vec<i32>,
    pub user_ids: Vec<i32>,
    pub submission_limit: i32,
}

///Struct for one contest and furthermore its submission time
#[derive(Clone)]
pub struct ContestInfo {
    pub id: Option<i32>,
    pub name: String,
    pub from: String,
    pub to: String,
    pub problem_ids: Vec<i32>,
    pub user_ids: Vec<i32>,
    pub submission_limit: i32,
    pub submission_time: i32,
}
