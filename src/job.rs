use serde::{Deserialize, Serialize};

///The information you need to provide when posting a job
#[derive(Clone, Serialize, Deserialize)]
pub struct PostJob {
    pub source_code: String,
    pub language: String,
    pub user_id: i32,
    pub contest_id: i32,
    pub problem_id: i32,
}

///State for the job
#[derive(Clone, Serialize, Deserialize)]
pub enum State {
    Queueing,
    Running,
    Finished,
    Canceled,
}

impl State {
    pub fn to_string(&self) -> String {
        match self {
            State::Queueing => "Queueing".to_string(),
            State::Canceled => "Canceled".to_string(),
            State::Running => "Running".to_string(),
            State::Finished => "Finished".to_string(),
        }
    }
}

///Result for one whole job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobResult {
    Waiting,
    Running,
    #[serde(rename = "Compilation Error")]
    CompilationError,
    Accepted,
    #[serde(rename = "Wrong Answer")]
    WrongAnswer,
    #[serde(rename = "Runtime Error")]
    RuntimeError,
    #[serde(rename = "Time Limit Exceeded")]
    TimeLimitExceeded,
    #[serde(rename = "Memory Limit Exceeded")]
    MemoryLimitExceeded,
    #[serde(rename = "System Error")]
    SystemError,
    #[serde(rename = "SPJ Error")]
    SpjError,
    Skipped,
}

impl JobResult {
    pub fn to_string(&self) -> String {
        match self {
            JobResult::Accepted => "Accepted".to_string(),
            JobResult::Running => "Running".to_string(),
            JobResult::Waiting => "Waiting".to_string(),
            JobResult::CompilationError => "Compilation Error".to_string(),
            JobResult::MemoryLimitExceeded => "Memory Limit Exceeded".to_string(),
            JobResult::RuntimeError => "Runtime Error".to_string(),
            JobResult::SystemError => "System Error".to_string(),
            JobResult::WrongAnswer => "Wrong Answer".to_string(),
            JobResult::SpjError => "Spj Error".to_string(),
            JobResult::Skipped => "Skipped".to_string(),
            JobResult::TimeLimitExceeded => "Time Limit Exceeded".to_string(),
        }
    }
}

///Result for the cases in one problem
#[derive(Clone, Serialize, Deserialize)]
pub enum CaseResult {
    Waiting,
    Running,
    Accepted,
    #[serde(rename = "Wrong Answer")]
    WrongAnswer,
    #[serde(rename = "Runtime Error")]
    RuntimeError,
    #[serde(rename = "Time Limit Exceeded")]
    TimeLimtExceeded,
    #[serde(rename = "Memory Limit Exceeded")]
    MemoryLimitExceeded,
    #[serde(rename = "System Error")]
    SystemError,
    #[serde(rename = "SPJ Error")]
    SpjError,
    #[serde(rename = "Compilation Error")]
    CompilationError,
    #[serde(rename = "Compilation Success")]
    CompilationSuccess,
    Skipped,
}

///The cases of the targeted problem the job posted
#[derive(Clone, Serialize, Deserialize)]
pub struct JobCase {
    pub id: i32,
    pub result: CaseResult,
    pub time: i32,
    pub memory: i32,
    pub info: String,
}

///The returning json value for the job posted, including online judging results and further information
#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: i32,
    pub created_time: String,
    pub updated_time: String,
    pub submission: PostJob,
    pub state: State,
    pub result: JobResult,
    pub score: f64,
    pub cases: Vec<JobCase>,
}

///The urls needed(optional) when requesting(get) one job information
#[derive(Clone, Serialize, Deserialize)]
pub struct JobQuery {
    pub user_id: Option<i32>,
    pub user_name: Option<i32>,
    pub contest_id: Option<i32>,
    pub problem_id: Option<i32>,
    pub language: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub state: Option<State>,
    pub result: Option<JobResult>,
}
