use crate::config::Config;
use actix_web::{
    get, middleware::Logger, post, put, web, App, HttpResponse, HttpServer, Responder,
};
use chrono::prelude::*;
use clap::Parser;
use env_logger;
use log;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{
    fs,
    fs::File,
    process::{Command, Stdio},
};
use tempfile::tempdir;
mod config;
mod contest;
mod job;
mod user;
use config::*;
use contest::*;
use job::*;
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use user::*;
use wait_timeout::ChildExt;

//List of jobs
lazy_static! {
    static ref JOBLIST: Arc<Mutex<Vec<Job>>> = Arc::new(Mutex::new(Vec::new()));
}

//List of users
lazy_static! {
    static ref USERLIST: Arc<Mutex<Vec<User>>> = Arc::new(Mutex::new(Vec::new()));
}

//List of contests
lazy_static! {
    static ref CONTESTLIST: Arc<Mutex<Vec<Contest>>> = Arc::new(Mutex::new(Vec::new()));
}

//List of contests and more info
lazy_static! {
    static ref CONTESTINFOLIST: Arc<Mutex<Vec<ContestInfo>>> = Arc::new(Mutex::new(Vec::new()));
}

///Structure for errors(if accurs)
#[derive(Clone, Serialize, Deserialize)]
struct Error {
    code: i32,
    reason: String,
    message: String,
}

///Arguments
#[derive(Parser, Deserialize, Serialize)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short = 'c', long, value_parser)]
    config: String,
    #[clap(short = 'f', long = "flush-data", action)]
    flushdata: bool,
}

///Default greeting function
#[get("/hello/{name}")]
async fn greet(name: web::Path<String>) -> impl Responder {
    log::info!(target: "greet_handler", "Greeting {}", name);
    format!("Hello {name}!")
}

///Post one job for online judging
#[post("/jobs")]
async fn post_jobs(body: web::Json<PostJob>, config: web::Data<Config>) -> impl Responder {
    let mut target_problem: Problem = Problem {
        id: 0,
        name: String::new(),
        ty: ProblemType::Standard,
        misc: None,
        cases: Vec::new(),
    };
    let mut target_language: Language = Language {
        name: "Rust".to_string(),
        file_name: String::new(),
        command: Vec::new(),
    };
    let lock = USERLIST.lock().unwrap(); //判断用户是否存在
    let mut i = 0;
    let mut flag_user = false;
    while i < lock.len() {
        if let Some(id) = lock[i].id {
            if id == body.user_id {
                flag_user = true;
                break;
            }
        }
        i += 1;
    }
    drop(lock);
    if !flag_user {
        HttpResponse::NotFound().json(Error {
            code: 3,
            reason: "ERR_NOT_FOUND".to_string(),
            message: "User not found".to_string(),
        })
    } else {
        //判断语言是否存在
        let mut flag = false;
        for language in &config.languages {
            if language.name == body.language {
                flag = true;
                target_language = language.clone();
                break;
            }
        }
        if !flag {
            HttpResponse::NotFound().json(Error {
                code: 3,
                reason: "ERR_NOT_FOUND".to_string(),
                message: "Language not found".to_string(),
            })
        } else {
            //判断题目是否在配置中
            for problem in &config.problems {
                if body.problem_id == problem.id {
                    flag = true;
                    target_problem = problem.clone();
                    break;
                }
            }
            if !flag {
                HttpResponse::NotFound().json(Error {
                    code: 3,
                    reason: "ERR_NOT_FOUND".to_string(),
                    message: "Problem not found".to_string(),
                })
            } else {
                //判断比赛相关
                let mut sub_time = 1;
                let lock = CONTESTINFOLIST.lock().unwrap();
                if body.contest_id as usize > lock.len() {
                    drop(lock);
                    HttpResponse::NotFound().json(Error {
                        code: 3,
                        reason: "ERR_NOT_FOUND".to_string(),
                        message: "Contest not found".to_string(),
                    })
                } else {
                    let mut flag = true; //判断是否返回400
                    if body.contest_id != 0 {
                        let target_contest = lock[body.contest_id as usize - 1].clone();
                        sub_time = target_contest.submission_time;
                        drop(lock);
                        if !target_contest.user_ids.contains(&body.user_id) {
                            flag = false;
                        }
                        if !target_contest.problem_ids.contains(&body.problem_id) {
                            flag = false;
                        }
                        let start_time = Utc
                            .datetime_from_str(
                                &target_contest.from.clone()[..],
                                "%Y-%m-%dT%H:%M:%S%.3fZ",
                            )
                            .unwrap();
                        let end_time = Utc
                            .datetime_from_str(
                                &target_contest.to.clone()[..],
                                "%Y-%m-%dT%H:%M:%S%.3fZ",
                            )
                            .unwrap();
                        if Utc::now() < start_time || Utc::now() > end_time {
                            flag = false;
                        }
                    } else {
                        drop(lock);
                    }
                    if !flag {
                        HttpResponse::BadRequest().json(Error {
                            code: 1,
                            reason: "ERR_INVALID_ARGUMENT".to_string(),
                            message: "Contest bad request".to_string(),
                        })
                    } else if sub_time == 0 {
                        HttpResponse::BadRequest().json(Error {
                            code: 4,
                            reason: "ERR_RATE_LIMIT".to_string(),
                            message: "Exceeding submission limit".to_string(),
                        })
                    } else {
                        //全部条件检查完毕无误
                        if body.contest_id != 0 {
                            //提交次数减去
                            let mut lock = CONTESTINFOLIST.lock().unwrap();
                            lock[body.contest_id as usize - 1].submission_time -= 1;
                            drop(lock);
                        }
                        let lock = JOBLIST.lock().unwrap();
                        let test_id = lock.len() as i32;
                        drop(lock);
                        let mut job: Job = Job {
                            //待返回响应的Job类型
                            id: test_id,
                            created_time: Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                            updated_time: Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                            submission: PostJob {
                                source_code: body.source_code.clone(),
                                language: body.language.clone(),
                                user_id: body.user_id,
                                contest_id: body.contest_id,
                                problem_id: body.problem_id,
                            },
                            state: State::Queueing,
                            result: JobResult::Waiting,
                            score: 0.0,
                            cases: Vec::new(),
                        };
                        job.cases.push(JobCase {
                            id: 0,
                            result: CaseResult::Waiting,
                            time: 0,
                            memory: 0,
                            info: String::new(),
                        });
                        let mut case_cnt = 1;
                        for _case in &target_problem.cases {
                            job.cases.push(JobCase {
                                id: case_cnt,
                                result: CaseResult::Waiting,
                                time: 0,
                                memory: 0,
                                info: String::new(),
                            });
                            case_cnt += 1;
                        } //Job初始化
                          //编译准备工作
                        let file_name = &target_language.file_name[..];
                        let program = &target_language.command[0][..];
                        let dir = tempdir().unwrap(); //临时目录
                        let pro_path = dir.path().join(file_name);
                        let pro_path_str = dir //目标语言文件目录
                            .path()
                            .join(file_name)
                            .as_os_str()
                            .to_str()
                            .unwrap()
                            .to_string();
                        let exe_path_str = dir //可执行文件目录
                            .path()
                            .join("test")
                            .as_os_str()
                            .to_str()
                            .unwrap()
                            .to_string();
                        let mut file = File::create(&pro_path).unwrap();
                        writeln!(file, "{}", body.source_code.clone()).unwrap(); //source_code写入文件
                        let mut args: Vec<String> = Vec::new();
                        for command in &target_language.command[1..] {
                            if command == &"%INPUT%".to_string() {
                                args.push(pro_path_str.clone());
                            } else if command == &"%OUTPUT%".to_string() {
                                args.push(exe_path_str.clone());
                            } else {
                                args.push(command.clone());
                            }
                        }
                        //开始编译
                        job.state = State::Running;
                        job.result = JobResult::Running;
                        job.cases[0].result = CaseResult::Running;
                        let now = Instant::now();
                        let status = Command::new(program).args(args).status();
                        let new_now = Instant::now();
                        if let Some(dur) = new_now.checked_duration_since(now) {
                            job.cases[0].time = (dur.as_secs_f64() * 1000000.0) as i32;
                        }
                        let mut finished = false; //结束标志
                        match status {
                            Ok(exitstatus) => {
                                if let Some(code) = exitstatus.code() {
                                    if code == 0 {
                                        job.cases[0].result = CaseResult::CompilationSuccess;
                                    } else {
                                        //编译错误
                                        job.cases[0].result = CaseResult::CompilationError;
                                        job.result = JobResult::CompilationError;
                                        job.state = State::Finished;
                                        finished = true;
                                    }
                                }
                            }
                            Err(_err) => {
                                job.cases[0].result = CaseResult::CompilationError;
                                job.result = JobResult::CompilationError;
                                job.state = State::Finished;
                                finished = true;
                            }
                        }
                        if finished {
                            //编译错误
                            let mut lock = JOBLIST.lock().unwrap();
                            lock.push(job.clone());
                            let mut jobs: Vec<Job> = Vec::new();
                            let mut i = 0;
                            while i < lock.len() {
                                jobs.push(lock[i].clone());
                                i += 1;
                            }
                            drop(lock);
                            //写入文件持久化存储
                            let out = std::fs::File::create("src/jobs.json").unwrap();
                            serde_json::to_writer(out, &jobs).unwrap();
                            HttpResponse::Ok().json(job)
                        } else {
                            //编译成功，开始检查
                            let mut flag_pack = false;
                            if let Some(misc) = target_problem.misc.clone() {
                                if let Some(batches) = misc.packing.clone() {
                                    //打包测试
                                    flag_pack = true;
                                    let mut case_cnt = 1;
                                    let mut batch_cnt = 0;
                                    let batch_tot = batches.len();
                                    while case_cnt <= target_problem.cases.len() {
                                        let case = target_problem.cases[case_cnt - 1].clone();
                                        let output_path = dir.path().join("test.out");
                                        let out_file = File::create(&output_path).unwrap();
                                        let in_file = File::open(case.input_file.clone()).unwrap();
                                        let mut tle = false; //是否超时判断
                                        let now = Instant::now();
                                        let mut child = Command::new(format!("{}", exe_path_str))
                                            .stdin(Stdio::from(in_file))
                                            .stdout(Stdio::from(out_file))
                                            .stderr(Stdio::null())
                                            .spawn()
                                            .unwrap();
                                        match child
                                            .wait_timeout(Duration::from_millis(
                                                (case.time_limit / 1000 + 500) as u64,
                                            ))
                                            .unwrap()
                                        {
                                            Some(_status) => {}
                                            None => {
                                                tle = true;
                                                child.kill().unwrap();
                                            }
                                        }
                                        let new_now = Instant::now();
                                        if tle {
                                            //超时！
                                            if let Some(dur) = new_now.checked_duration_since(now) {
                                                job.cases[case_cnt].time =
                                                    (dur.as_secs_f64() * 1000000.0) as i32;
                                            }
                                            job.cases[case_cnt].result =
                                                CaseResult::TimeLimtExceeded;
                                            match job.result {
                                                JobResult::Running => {
                                                    job.result = JobResult::TimeLimitExceeded;
                                                }
                                                _ => {}
                                            }
                                            //该batch的所有其他均为Skipped
                                            for i in batches[batch_cnt].clone() {
                                                if i as usize > case_cnt {
                                                    job.cases[i as usize].result =
                                                        CaseResult::Skipped;
                                                }
                                            }
                                            batch_cnt += 1;
                                            if batch_cnt >= batch_tot {
                                                break;
                                            } else {
                                                case_cnt = batches[batch_cnt][0] as usize;
                                            }
                                            continue;
                                        } //未超时
                                        let out_file = File::create(&output_path).unwrap();
                                        let in_file = File::open(case.input_file.clone()).unwrap();
                                        let now = Instant::now();
                                        let status = Command::new(format!("{}", exe_path_str))
                                            .stdin(Stdio::from(in_file))
                                            .stdout(Stdio::from(out_file))
                                            .stderr(Stdio::null())
                                            .status()
                                            .unwrap();
                                        let new_now = Instant::now();
                                        if let Some(dur) = new_now.checked_duration_since(now) {
                                            job.cases[case_cnt].time =
                                                (dur.as_secs_f64() * 1000000.0) as i32;
                                        }
                                        if let Some(x) = status.code() {
                                            if x != 0 {
                                                //RE!
                                                job.cases[case_cnt].result =
                                                    CaseResult::RuntimeError;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::RuntimeError;
                                                    }
                                                    _ => {}
                                                }
                                                //该batch的所有其他均为Skipped
                                                for i in batches[batch_cnt].clone() {
                                                    if i as usize > case_cnt {
                                                        job.cases[i as usize].result =
                                                            CaseResult::Skipped;
                                                    }
                                                }
                                                batch_cnt += 1;
                                                if batch_cnt >= batch_tot {
                                                    break;
                                                } else {
                                                    case_cnt = batches[batch_cnt][0] as usize;
                                                }
                                                continue;
                                            }
                                        }
                                        match target_problem.ty {
                                            ProblemType::Standard => {
                                                //标准模式
                                                let origin_file: String = std::fs::read_to_string(
                                                    case.answer_file.clone(),
                                                )
                                                .unwrap()
                                                .trim()
                                                .to_string();
                                                let origin: Vec<String> = origin_file
                                                    .lines()
                                                    .into_iter()
                                                    .map(move |str| {
                                                        str.to_string().trim().to_string()
                                                    })
                                                    .collect();
                                                let temp_file: String = std::fs::read_to_string(
                                                    output_path.as_os_str().to_str().unwrap(),
                                                )
                                                .unwrap()
                                                .trim()
                                                .to_string();
                                                let temp: Vec<String> = temp_file
                                                    .lines()
                                                    .into_iter()
                                                    .map(move |str| {
                                                        str.to_string().trim().to_string()
                                                    })
                                                    .collect();
                                                let mut flag = true;
                                                if origin.len() != temp.len() {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::WrongAnswer;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result = JobResult::WrongAnswer;
                                                        }
                                                        _ => {}
                                                    }
                                                    flag = false;
                                                } else {
                                                    //逐行比对
                                                    let mut i = 0;
                                                    while i < origin.len() {
                                                        if origin[i] != temp[i] {
                                                            job.cases[case_cnt].result =
                                                                CaseResult::WrongAnswer;
                                                            match job.result {
                                                                JobResult::Running => {
                                                                    job.result =
                                                                        JobResult::WrongAnswer;
                                                                }
                                                                _ => {}
                                                            }
                                                            flag = false;
                                                            break;
                                                        }
                                                        i += 1;
                                                    }
                                                }
                                                if flag {
                                                    //测试点通过
                                                    job.cases[case_cnt].result =
                                                        CaseResult::Accepted;
                                                    //如果直到最后测试点均通过
                                                    if case_cnt
                                                        == batches[batch_cnt]
                                                            [batches[batch_cnt].len() - 1]
                                                            as usize
                                                    {
                                                        for case_index in batches[batch_cnt].clone()
                                                        {
                                                            job.score += target_problem.cases
                                                                [case_index as usize - 1]
                                                                .score;
                                                        }
                                                        batch_cnt += 1;
                                                        if batch_cnt >= batch_tot {
                                                            break;
                                                        } else {
                                                            case_cnt =
                                                                batches[batch_cnt][0] as usize;
                                                        }
                                                        continue;
                                                    } else {
                                                        case_cnt += 1;
                                                        continue;
                                                    }
                                                } else {
                                                    //该batch的所有其他均为Skipped
                                                    for i in batches[batch_cnt].clone() {
                                                        if i as usize > case_cnt {
                                                            job.cases[i as usize].result =
                                                                CaseResult::Skipped;
                                                        }
                                                    }
                                                    batch_cnt += 1;
                                                    if batch_cnt >= batch_tot {
                                                        break;
                                                    } else {
                                                        case_cnt = batches[batch_cnt][0] as usize;
                                                    }
                                                    continue;
                                                }
                                            }
                                            ProblemType::Strict => {
                                                //严格模式，直接比较字符串
                                                let origin: String = std::fs::read_to_string(
                                                    case.answer_file.clone(),
                                                )
                                                .unwrap();
                                                let temp: String = std::fs::read_to_string(
                                                    output_path.as_os_str().to_str().unwrap(),
                                                )
                                                .unwrap();
                                                if origin != temp {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::WrongAnswer;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result = JobResult::WrongAnswer;
                                                        }
                                                        _ => {}
                                                    }
                                                    //该batch的所有其他均为Skipped
                                                    for i in batches[batch_cnt].clone() {
                                                        if i as usize > case_cnt {
                                                            job.cases[i as usize].result =
                                                                CaseResult::Skipped;
                                                        }
                                                    }
                                                    batch_cnt += 1;
                                                    if batch_cnt >= batch_tot {
                                                        break;
                                                    } else {
                                                        case_cnt = batches[batch_cnt][0] as usize;
                                                    }
                                                    continue;
                                                } else {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::Accepted;
                                                    //如果直到最后测试点均通过
                                                    if case_cnt
                                                        == batches[batch_cnt]
                                                            [batches[batch_cnt].len() - 1]
                                                            as usize
                                                    {
                                                        for case_index in batches[batch_cnt].clone()
                                                        {
                                                            job.score += target_problem.cases
                                                                [case_index as usize - 1]
                                                                .score;
                                                        }
                                                        batch_cnt += 1;
                                                        if batch_cnt >= batch_tot {
                                                            break;
                                                        } else {
                                                            case_cnt =
                                                                batches[batch_cnt][0] as usize;
                                                        }
                                                        continue;
                                                    } else {
                                                        case_cnt += 1;
                                                        continue;
                                                    }
                                                }
                                            }
                                            ProblemType::Spj => {
                                                //Special Judge
                                                if let Some(misc) = target_problem.misc.clone() {
                                                    if let Some(command) =
                                                        misc.special_judge.clone()
                                                    {
                                                        let program = command[0].clone();
                                                        let mut args: Vec<String> = Vec::new();
                                                        for arg in &command[1..] {
                                                            if arg == &"%OUTPUT%".to_string() {
                                                                args.push(
                                                                    output_path
                                                                        .as_os_str()
                                                                        .to_str()
                                                                        .unwrap()
                                                                        .to_string()
                                                                        .clone(),
                                                                );
                                                            } else if arg == &"%ANSWER%".to_string()
                                                            {
                                                                args.push(case.answer_file.clone());
                                                            } else {
                                                                args.push(arg.clone());
                                                            }
                                                        }
                                                        let feedback =
                                                            File::create("src/feedback.out")
                                                                .unwrap();
                                                        let _status = Command::new(program)
                                                            .args(args)
                                                            .stdout(Stdio::from(feedback))
                                                            .status()
                                                            .unwrap();
                                                        let result: String =
                                                            std::fs::read_to_string(
                                                                "src/feedback.out",
                                                            )
                                                            .unwrap();
                                                        let temp: Vec<String> = result
                                                            .lines()
                                                            .into_iter()
                                                            .map(move |str| {
                                                                str.to_string().trim().to_string()
                                                            })
                                                            .collect();
                                                        if temp[0] == "Accepted".to_string() {
                                                            job.cases[case_cnt].result =
                                                                CaseResult::Accepted;
                                                            job.score += case.score;
                                                        } else if temp[0]
                                                            == "Wrong Answer".to_string()
                                                        {
                                                            job.cases[case_cnt].result =
                                                                CaseResult::WrongAnswer;
                                                            match job.result {
                                                                JobResult::Running => {
                                                                    job.result =
                                                                        JobResult::WrongAnswer;
                                                                }
                                                                _ => {}
                                                            }
                                                        }
                                                        else {//输出信息不符合要求
                                                            job.cases[case_cnt].result =
                                                                CaseResult::SpjError;
                                                            match job.result {
                                                                JobResult::Running => {
                                                                    job.result =
                                                                        JobResult::SpjError;
                                                                }
                                                                _ => {}
                                                            }
                                                        }
                                                        job.cases[case_cnt].info = temp[1].clone();
                                                    }
                                                }
                                            }
                                            ProblemType::DynamicRanking => {
                                                //竞争得分
                                                let mut ratio = 0.0;
                                                if let Some(misc) = target_problem.misc.clone() {
                                                    if let Some(r) =
                                                        misc.dynamic_ranking_ratio.clone()
                                                    {
                                                        ratio = 1.0 - r;
                                                    }
                                                }
                                                let origin_file: String = std::fs::read_to_string(
                                                    case.answer_file.clone(),
                                                )
                                                .unwrap()
                                                .trim()
                                                .to_string();
                                                let origin: Vec<String> = origin_file
                                                    .lines()
                                                    .into_iter()
                                                    .map(move |str| {
                                                        str.to_string().trim().to_string()
                                                    })
                                                    .collect();
                                                let temp_file: String = std::fs::read_to_string(
                                                    output_path.as_os_str().to_str().unwrap(),
                                                )
                                                .unwrap()
                                                .trim()
                                                .to_string();
                                                let temp: Vec<String> = temp_file
                                                    .lines()
                                                    .into_iter()
                                                    .map(move |str| {
                                                        str.to_string().trim().to_string()
                                                    })
                                                    .collect();
                                                let mut flag = true;
                                                if origin.len() != temp.len() {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::WrongAnswer;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result = JobResult::WrongAnswer;
                                                        }
                                                        _ => {}
                                                    }
                                                    flag = false;
                                                } else {
                                                    //逐行比对
                                                    let mut i = 0;
                                                    while i < origin.len() {
                                                        if origin[i] != temp[i] {
                                                            job.cases[case_cnt].result =
                                                                CaseResult::WrongAnswer;
                                                            match job.result {
                                                                JobResult::Running => {
                                                                    job.result =
                                                                        JobResult::WrongAnswer;
                                                                }
                                                                _ => {}
                                                            }
                                                            flag = false;
                                                            break;
                                                        }
                                                        i += 1;
                                                    }
                                                }
                                                if flag {
                                                    //测试点通过，增加基础得分
                                                    job.cases[case_cnt].result =
                                                        CaseResult::Accepted;
                                                    job.score += case.score * ratio;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if !flag_pack {
                                //没有打包
                                let mut case_cnt = 0;
                                for case in &target_problem.cases {
                                    case_cnt += 1;
                                    let output_path = dir.path().join("test.out");
                                    let out_file = File::create(&output_path).unwrap();
                                    let in_file = File::open(case.input_file.clone()).unwrap();
                                    let mut tle = false; //是否超时判断
                                    let now = Instant::now();
                                    let mut child = Command::new(format!("{}", exe_path_str))
                                        .stdin(Stdio::from(in_file))
                                        .stdout(Stdio::from(out_file))
                                        .stderr(Stdio::null())
                                        .spawn()
                                        .unwrap();
                                    match child
                                        .wait_timeout(Duration::from_millis(
                                            (case.time_limit / 1000 + 500) as u64,
                                        ))
                                        .unwrap()
                                    {
                                        Some(_status) => {}
                                        None => {
                                            tle = true;
                                            child.kill().unwrap();
                                        }
                                    }
                                    let new_now = Instant::now();
                                    if tle {
                                        //超时！
                                        if let Some(dur) = new_now.checked_duration_since(now) {
                                            job.cases[case_cnt].time =
                                                (dur.as_secs_f64() * 1000000.0) as i32;
                                        }
                                        job.cases[case_cnt].result = CaseResult::TimeLimtExceeded;
                                        match job.result {
                                            JobResult::Running => {
                                                job.result = JobResult::TimeLimitExceeded;
                                            }
                                            _ => {}
                                        }
                                        continue;
                                    }
                                    let out_file = File::create(&output_path).unwrap();
                                    let in_file = File::open(case.input_file.clone()).unwrap();
                                    let now = Instant::now();
                                    let status = Command::new(format!("{}", exe_path_str))
                                        .stdin(Stdio::from(in_file))
                                        .stdout(Stdio::from(out_file))
                                        .stderr(Stdio::null())
                                        .status()
                                        .unwrap();
                                    let new_now = Instant::now();
                                    if let Some(dur) = new_now.checked_duration_since(now) {
                                        job.cases[case_cnt].time =
                                            (dur.as_secs_f64() * 1000000.0) as i32;
                                    }
                                    if let Some(x) = status.code() {
                                        if x != 0 {
                                            job.cases[case_cnt].result = CaseResult::RuntimeError;
                                            match job.result {
                                                JobResult::Running => {
                                                    job.result = JobResult::RuntimeError;
                                                }
                                                _ => {}
                                            }
                                            continue;
                                        }
                                    }
                                    match target_problem.ty {
                                        ProblemType::Standard => {
                                            //标准模式
                                            let origin_file: String =
                                                std::fs::read_to_string(case.answer_file.clone())
                                                    .unwrap()
                                                    .trim()
                                                    .to_string();
                                            let origin: Vec<String> = origin_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let temp_file: String = std::fs::read_to_string(
                                                output_path.as_os_str().to_str().unwrap(),
                                            )
                                            .unwrap()
                                            .trim()
                                            .to_string();
                                            let temp: Vec<String> = temp_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let mut flag = true;
                                            if origin.len() != temp.len() {
                                                job.cases[case_cnt].result =
                                                    CaseResult::WrongAnswer;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::WrongAnswer;
                                                    }
                                                    _ => {}
                                                }
                                                flag = false;
                                            } else {
                                                //逐行比对
                                                let mut i = 0;
                                                while i < origin.len() {
                                                    if origin[i] != temp[i] {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::WrongAnswer;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result = JobResult::WrongAnswer;
                                                            }
                                                            _ => {}
                                                        }
                                                        flag = false;
                                                        break;
                                                    }
                                                    i += 1;
                                                }
                                            }
                                            if flag {
                                                //测试点通过
                                                job.cases[case_cnt].result = CaseResult::Accepted;
                                                job.score += case.score;
                                            }
                                        }
                                        ProblemType::Strict => {
                                            //严格模式，直接比较字符串
                                            let origin: String =
                                                std::fs::read_to_string(case.answer_file.clone())
                                                    .unwrap();
                                            let temp: String = std::fs::read_to_string(
                                                output_path.as_os_str().to_str().unwrap(),
                                            )
                                            .unwrap();
                                            if origin != temp {
                                                job.cases[case_cnt].result =
                                                    CaseResult::WrongAnswer;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::WrongAnswer;
                                                    }
                                                    _ => {}
                                                }
                                            } else {
                                                job.cases[case_cnt].result = CaseResult::Accepted;
                                                job.score += case.score;
                                            }
                                        }
                                        ProblemType::Spj => {
                                            //Special Judge
                                            if let Some(misc) = target_problem.misc.clone() {
                                                if let Some(command) = misc.special_judge.clone() {
                                                    let program = command[0].clone();
                                                    let mut args: Vec<String> = Vec::new();
                                                    for arg in &command[1..] {
                                                        if arg == &"%OUTPUT%".to_string() {
                                                            args.push(
                                                                output_path
                                                                    .as_os_str()
                                                                    .to_str()
                                                                    .unwrap()
                                                                    .to_string()
                                                                    .clone(),
                                                            );
                                                        } else if arg == &"%ANSWER%".to_string() {
                                                            args.push(case.answer_file.clone());
                                                        } else {
                                                            args.push(arg.clone());
                                                        }
                                                    }
                                                    let feedback =
                                                        File::create("src/feedback.out").unwrap();
                                                    let _status = Command::new(program)
                                                        .args(args)
                                                        .stdout(Stdio::from(feedback))
                                                        .status()
                                                        .unwrap();
                                                    let result: String =
                                                        std::fs::read_to_string("src/feedback.out")
                                                            .unwrap();
                                                    let temp: Vec<String> = result
                                                        .lines()
                                                        .into_iter()
                                                        .map(move |str| {
                                                            str.to_string().trim().to_string()
                                                        })
                                                        .collect();
                                                    if temp[0] == "Accepted".to_string() {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::Accepted;
                                                        job.score += case.score;
                                                    } else if temp[0] == "Wrong Answer".to_string()
                                                    {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::WrongAnswer;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result = JobResult::WrongAnswer;
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    else {//输出信息不符合要求
                                                        job.cases[case_cnt].result =
                                                            CaseResult::SpjError;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result =
                                                                    JobResult::SpjError;
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    job.cases[case_cnt].info = temp[1].clone();
                                                }
                                            }
                                        }
                                        ProblemType::DynamicRanking => {
                                            //竞争得分
                                            let mut ratio = 0.0;
                                            if let Some(misc) = target_problem.misc.clone() {
                                                if let Some(r) = misc.dynamic_ranking_ratio.clone()
                                                {
                                                    ratio = 1.0 - r;
                                                }
                                            }
                                            let origin_file: String =
                                                std::fs::read_to_string(case.answer_file.clone())
                                                    .unwrap()
                                                    .trim()
                                                    .to_string();
                                            let origin: Vec<String> = origin_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let temp_file: String = std::fs::read_to_string(
                                                output_path.as_os_str().to_str().unwrap(),
                                            )
                                            .unwrap()
                                            .trim()
                                            .to_string();
                                            let temp: Vec<String> = temp_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let mut flag = true;
                                            if origin.len() != temp.len() {
                                                job.cases[case_cnt].result =
                                                    CaseResult::WrongAnswer;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::WrongAnswer;
                                                    }
                                                    _ => {}
                                                }
                                                flag = false;
                                            } else {
                                                //逐行比对
                                                let mut i = 0;
                                                while i < origin.len() {
                                                    if origin[i] != temp[i] {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::WrongAnswer;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result = JobResult::WrongAnswer;
                                                            }
                                                            _ => {}
                                                        }
                                                        flag = false;
                                                        break;
                                                    }
                                                    i += 1;
                                                }
                                            }
                                            if flag {
                                                //测试点通过，增加基础得分
                                                job.cases[case_cnt].result = CaseResult::Accepted;
                                                job.score += case.score * ratio;
                                            }
                                        }
                                    }
                                }
                            }
                            //全部测试完成
                            job.state = State::Finished;
                            match job.result {
                                JobResult::Running => {
                                    job.result = JobResult::Accepted;
                                }
                                _ => {}
                            }
                            let mut lock = JOBLIST.lock().unwrap();
                            lock.push(job.clone());
                            let mut jobs: Vec<Job> = Vec::new();
                            let mut i = 0;
                            while i < lock.len() {
                                jobs.push(lock[i].clone());
                                i += 1;
                            }
                            drop(lock);
                            //写入文件持久化存储
                            let out = std::fs::File::create("src/jobs.json").unwrap();
                            serde_json::to_writer(out, &jobs).unwrap();
                            HttpResponse::Ok().json(job)
                        }
                    }
                }
            }
        }
    }
}

///Get the whole job list
#[get("/jobs")]
async fn get_jobs(info: web::Query<JobQuery>) -> impl Responder {
    let lock = JOBLIST.lock().unwrap();
    let mut jobs: Vec<Job> = Vec::new();
    let mut i = 0;
    while i < lock.len() {
        jobs.push(lock[i].clone());
        i += 1;
    }
    drop(lock);
    if let Some(id) = info.problem_id {
        let mut i = 0;
        while i < jobs.len() {
            if jobs[i].submission.problem_id != id {
                jobs.remove(i);
            } else {
                i += 1;
            }
        }
    }
    if let Some(language) = info.language.clone() {
        let mut i = 0;
        while i < jobs.len() {
            if jobs[i].submission.language != language {
                jobs.remove(i);
            } else {
                i += 1;
            }
        }
    }
    if let Some(state) = info.state.clone() {
        let mut i = 0;
        while i < jobs.len() {
            if jobs[i].state.to_string() != state.to_string() {
                jobs.remove(i);
            } else {
                i += 1;
            }
        }
    }
    if let Some(result) = info.result.clone() {
        let mut i = 0;
        while i < jobs.len() {
            if jobs[i].result.to_string() != result.to_string() {
                jobs.remove(i);
            } else {
                i += 1;
            }
        }
    }
    HttpResponse::Ok().json(jobs)
}

///Get the job by its jobid
#[get("/jobs/{id}")]
async fn get_job_id(id: web::Path<i32>) -> impl Responder {
    let lock = JOBLIST.lock().unwrap();
    if id.abs() as usize >= lock.len() {
        HttpResponse::NotFound().json(Error {
            code: 3,
            reason: "ERR_NOT_FOUND".to_string(),
            message: format!("Job {} not found", id.abs()),
        })
    } else {
        let job: Job = lock[id.abs() as usize].clone();
        HttpResponse::Ok().json(job)
    }
}

///Rejudge one job
#[put("/jobs/{id}")]
async fn put_job_id(id: web::Path<i32>, config: web::Data<Config>) -> impl Responder {
    let lock = JOBLIST.lock().unwrap();
    if id.abs() as usize >= lock.len() {
        drop(lock);
        HttpResponse::NotFound().json(Error {
            code: 3,
            reason: "ERR_NOT_FOUND".to_string(),
            message: format!("Job {} not found", id.abs()),
        })
    } else {
        let targetjob: Job = lock[id.abs() as usize].clone();
        drop(lock);
        let body = targetjob.submission.clone();
        let mut target_problem: Problem = Problem {
            id: 0,
            name: String::new(),
            ty: ProblemType::Standard,
            misc: None,
            cases: Vec::new(),
        };
        let mut target_language: Language = Language {
            name: "Rust".to_string(),
            file_name: String::new(),
            command: Vec::new(),
        };
        //判断语言是否存在
        let mut flag = false;
        for language in &config.languages {
            if language.name == body.language {
                flag = true;
                target_language = language.clone();
                break;
            }
        }
        if !flag {
            HttpResponse::NotFound().json(Error {
                code: 3,
                reason: "ERR_NOT_FOUND".to_string(),
                message: "Language not found".to_string(),
            })
        } else {
            //判断题目是否在配置中
            for problem in &config.problems {
                if body.problem_id == problem.id {
                    flag = true;
                    target_problem = problem.clone();
                    break;
                }
            }
            if !flag {
                HttpResponse::NotFound().json(Error {
                    code: 3,
                    reason: "ERR_NOT_FOUND".to_string(),
                    message: "Problem not found".to_string(),
                })
            } else {
                //检查完毕无误
                let lock = JOBLIST.lock().unwrap();
                let test_id = id.abs();
                if lock[test_id as usize].state.to_string() != "Finished".to_string() {
                    HttpResponse::BadRequest().json(Error {
                        code: 2,
                        reason: "ERR_INVALID_STATE".to_string(),
                        message: format!("Job {} not finished", id),
                    })
                } else {
                    let mut job: Job = Job {
                        id: test_id,
                        created_time: lock[test_id as usize].created_time.clone(),
                        updated_time: Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                        submission: PostJob {
                            source_code: body.source_code.clone(),
                            language: body.language.clone(),
                            user_id: body.user_id,
                            contest_id: body.contest_id,
                            problem_id: body.problem_id,
                        },
                        state: State::Queueing,
                        result: JobResult::Waiting,
                        score: 0.0,
                        cases: Vec::new(),
                    };
                    drop(lock);
                    job.cases.push(JobCase {
                        id: 0,
                        result: CaseResult::Waiting,
                        time: 0,
                        memory: 0,
                        info: String::new(),
                    });
                    let mut case_cnt = 1;
                    for _case in &target_problem.cases {
                        job.cases.push(JobCase {
                            id: case_cnt,
                            result: CaseResult::Waiting,
                            time: 0,
                            memory: 0,
                            info: String::new(),
                        });
                        case_cnt += 1;
                    } //Job初始化
                      //编译准备工作
                    let file_name = &target_language.file_name[..];
                    let program = &target_language.command[0][..];
                    let dir = tempdir().unwrap(); //临时目录
                    let pro_path = dir.path().join(file_name);
                    let pro_path_str = dir
                        .path()
                        .join(file_name)
                        .as_os_str()
                        .to_str()
                        .unwrap()
                        .to_string();
                    let exe_path_str = dir
                        .path()
                        .join("test")
                        .as_os_str()
                        .to_str()
                        .unwrap()
                        .to_string();
                    let mut file = File::create(&pro_path).unwrap();
                    writeln!(file, "{}", body.source_code.clone()).unwrap();
                    let mut args: Vec<String> = Vec::new();
                    for command in &target_language.command[1..] {
                        if command == &"%INPUT%".to_string() {
                            args.push(pro_path_str.clone());
                        } else if command == &"%OUTPUT%".to_string() {
                            args.push(exe_path_str.clone());
                        } else {
                            args.push(command.clone());
                        }
                    }
                    //开始编译
                    job.state = State::Running;
                    job.result = JobResult::Running;
                    job.cases[0].result = CaseResult::Running;
                    let now = Instant::now();
                    let status = Command::new(program).args(args).status();
                    let new_now = Instant::now();
                    if let Some(dur) = new_now.checked_duration_since(now) {
                        job.cases[0].time = (dur.as_secs_f64() * 1000000.0) as i32;
                    }
                    let mut finished = false; //结束标志
                    match status {
                        Ok(exitstatus) => {
                            if let Some(code) = exitstatus.code() {
                                if code == 0 {
                                    job.cases[0].result = CaseResult::CompilationSuccess;
                                } else {
                                    //编译错误
                                    job.cases[0].result = CaseResult::CompilationError;
                                    job.result = JobResult::CompilationError;
                                    job.state = State::Finished;
                                    finished = true;
                                }
                            }
                        }
                        Err(_err) => {
                            job.cases[0].result = CaseResult::CompilationError;
                            job.result = JobResult::CompilationError;
                            job.state = State::Finished;
                            finished = true;
                        }
                    }
                    if finished {
                        let mut lock = JOBLIST.lock().unwrap();
                        lock[id.abs() as usize] = job.clone();
                        drop(lock);
                        HttpResponse::Ok().json(job)
                    } else {
                        //编译成功，开始检查
                        let mut flag_pack = false;
                        if let Some(misc) = target_problem.misc.clone() {
                            if let Some(batches) = misc.packing.clone() {
                                //打包测试
                                flag_pack = true;
                                let mut case_cnt = 1;
                                let mut batch_cnt = 0;
                                let batch_tot = batches.len();
                                while case_cnt <= target_problem.cases.len() {
                                    let case = target_problem.cases[case_cnt - 1].clone();
                                    let output_path = dir.path().join("test.out");
                                    let out_file = File::create(&output_path).unwrap();
                                    let in_file = File::open(case.input_file.clone()).unwrap();
                                    let mut tle = false; //是否超时判断
                                    let now = Instant::now();
                                    let mut child = Command::new(format!("{}", exe_path_str))
                                        .stdin(Stdio::from(in_file))
                                        .stdout(Stdio::from(out_file))
                                        .stderr(Stdio::null())
                                        .spawn()
                                        .unwrap();
                                    match child
                                        .wait_timeout(Duration::from_millis(
                                            (case.time_limit / 1000 + 500) as u64,
                                        ))
                                        .unwrap()
                                    {
                                        Some(_status) => {}
                                        None => {
                                            tle = true;
                                            child.kill().unwrap();
                                        }
                                    }
                                    let new_now = Instant::now();
                                    if tle {
                                        //超时！
                                        if let Some(dur) = new_now.checked_duration_since(now) {
                                            job.cases[case_cnt].time =
                                                (dur.as_secs_f64() * 1000000.0) as i32;
                                        }
                                        job.cases[case_cnt].result = CaseResult::TimeLimtExceeded;
                                        match job.result {
                                            JobResult::Running => {
                                                job.result = JobResult::TimeLimitExceeded;
                                            }
                                            _ => {}
                                        }
                                        //该batch的所有其他均为Skipped
                                        for i in batches[batch_cnt].clone() {
                                            if i as usize > case_cnt {
                                                job.cases[i as usize].result = CaseResult::Skipped;
                                            }
                                        }
                                        batch_cnt += 1;
                                        if batch_cnt >= batch_tot {
                                            break;
                                        } else {
                                            case_cnt = batches[batch_cnt][0] as usize;
                                        }
                                        continue;
                                    } //未超时
                                    let out_file = File::create(&output_path).unwrap();
                                    let in_file = File::open(case.input_file.clone()).unwrap();
                                    let now = Instant::now();
                                    let status = Command::new(format!("{}", exe_path_str))
                                        .stdin(Stdio::from(in_file))
                                        .stdout(Stdio::from(out_file))
                                        .stderr(Stdio::null())
                                        .status()
                                        .unwrap();
                                    let new_now = Instant::now();
                                    if let Some(dur) = new_now.checked_duration_since(now) {
                                        job.cases[case_cnt].time =
                                            (dur.as_secs_f64() * 1000000.0) as i32;
                                    }
                                    if let Some(x) = status.code() {
                                        if x != 0 {
                                            //RE!
                                            job.cases[case_cnt].result = CaseResult::RuntimeError;
                                            match job.result {
                                                JobResult::Running => {
                                                    job.result = JobResult::RuntimeError;
                                                }
                                                _ => {}
                                            }
                                            //该batch的所有其他均为Skipped
                                            for i in batches[batch_cnt].clone() {
                                                if i as usize > case_cnt {
                                                    job.cases[i as usize].result =
                                                        CaseResult::Skipped;
                                                }
                                            }
                                            batch_cnt += 1;
                                            if batch_cnt >= batch_tot {
                                                break;
                                            } else {
                                                case_cnt = batches[batch_cnt][0] as usize;
                                            }
                                            continue;
                                        }
                                    }
                                    match target_problem.ty {
                                        ProblemType::Standard => {
                                            //标准模式
                                            let origin_file: String =
                                                std::fs::read_to_string(case.answer_file.clone())
                                                    .unwrap()
                                                    .trim()
                                                    .to_string();
                                            let origin: Vec<String> = origin_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let temp_file: String = std::fs::read_to_string(
                                                output_path.as_os_str().to_str().unwrap(),
                                            )
                                            .unwrap()
                                            .trim()
                                            .to_string();
                                            let temp: Vec<String> = temp_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let mut flag = true;
                                            if origin.len() != temp.len() {
                                                job.cases[case_cnt].result =
                                                    CaseResult::WrongAnswer;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::WrongAnswer;
                                                    }
                                                    _ => {}
                                                }
                                                flag = false;
                                            } else {
                                                //逐行比对
                                                let mut i = 0;
                                                while i < origin.len() {
                                                    if origin[i] != temp[i] {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::WrongAnswer;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result = JobResult::WrongAnswer;
                                                            }
                                                            _ => {}
                                                        }
                                                        flag = false;
                                                        break;
                                                    }
                                                    i += 1;
                                                }
                                            }
                                            if flag {
                                                //测试点通过
                                                job.cases[case_cnt].result = CaseResult::Accepted;
                                                //如果直到最后测试点均通过
                                                if case_cnt
                                                    == batches[batch_cnt]
                                                        [batches[batch_cnt].len() - 1]
                                                        as usize
                                                {
                                                    for case_index in batches[batch_cnt].clone() {
                                                        job.score += target_problem.cases
                                                            [case_index as usize - 1]
                                                            .score;
                                                    }
                                                    batch_cnt += 1;
                                                    if batch_cnt >= batch_tot {
                                                        break;
                                                    } else {
                                                        case_cnt = batches[batch_cnt][0] as usize;
                                                    }
                                                    continue;
                                                } else {
                                                    case_cnt += 1;
                                                    continue;
                                                }
                                            } else {
                                                //该batch的所有其他均为Skipped
                                                for i in batches[batch_cnt].clone() {
                                                    if i as usize > case_cnt {
                                                        job.cases[i as usize].result =
                                                            CaseResult::Skipped;
                                                    }
                                                }
                                                batch_cnt += 1;
                                                if batch_cnt >= batch_tot {
                                                    break;
                                                } else {
                                                    case_cnt = batches[batch_cnt][0] as usize;
                                                }
                                                continue;
                                            }
                                        }
                                        ProblemType::Strict => {
                                            //严格模式，直接比较字符串
                                            let origin: String =
                                                std::fs::read_to_string(case.answer_file.clone())
                                                    .unwrap();
                                            let temp: String = std::fs::read_to_string(
                                                output_path.as_os_str().to_str().unwrap(),
                                            )
                                            .unwrap();
                                            if origin != temp {
                                                job.cases[case_cnt].result =
                                                    CaseResult::WrongAnswer;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::WrongAnswer;
                                                    }
                                                    _ => {}
                                                }
                                                //该batch的所有其他均为Skipped
                                                for i in batches[batch_cnt].clone() {
                                                    if i as usize > case_cnt {
                                                        job.cases[i as usize].result =
                                                            CaseResult::Skipped;
                                                    }
                                                }
                                                batch_cnt += 1;
                                                if batch_cnt >= batch_tot {
                                                    break;
                                                } else {
                                                    case_cnt = batches[batch_cnt][0] as usize;
                                                }
                                                continue;
                                            } else {
                                                job.cases[case_cnt].result = CaseResult::Accepted;
                                                //如果直到最后测试点均通过
                                                if case_cnt
                                                    == batches[batch_cnt]
                                                        [batches[batch_cnt].len() - 1]
                                                        as usize
                                                {
                                                    for case_index in batches[batch_cnt].clone() {
                                                        job.score += target_problem.cases
                                                            [case_index as usize - 1]
                                                            .score;
                                                    }
                                                    batch_cnt += 1;
                                                    if batch_cnt >= batch_tot {
                                                        break;
                                                    } else {
                                                        case_cnt = batches[batch_cnt][0] as usize;
                                                    }
                                                    continue;
                                                } else {
                                                    case_cnt += 1;
                                                    continue;
                                                }
                                            }
                                        }
                                        ProblemType::Spj => {
                                            //Special Judge
                                            if let Some(misc) = target_problem.misc.clone() {
                                                if let Some(command) = misc.special_judge.clone() {
                                                    let program = command[0].clone();
                                                    let mut args: Vec<String> = Vec::new();
                                                    for arg in &command[1..] {
                                                        if arg == &"%OUTPUT%".to_string() {
                                                            args.push(
                                                                output_path
                                                                    .as_os_str()
                                                                    .to_str()
                                                                    .unwrap()
                                                                    .to_string()
                                                                    .clone(),
                                                            );
                                                        } else if arg == &"%ANSWER%".to_string() {
                                                            args.push(case.answer_file.clone());
                                                        } else {
                                                            args.push(arg.clone());
                                                        }
                                                    }
                                                    let feedback =
                                                        File::create("src/feedback.out").unwrap();
                                                    let _status = Command::new(program)
                                                        .args(args)
                                                        .stdout(Stdio::from(feedback))
                                                        .status()
                                                        .unwrap();
                                                    let result: String =
                                                        std::fs::read_to_string("src/feedback.out")
                                                            .unwrap();
                                                    let temp: Vec<String> = result
                                                        .lines()
                                                        .into_iter()
                                                        .map(move |str| {
                                                            str.to_string().trim().to_string()
                                                        })
                                                        .collect();
                                                    if temp[0] == "Accepted".to_string() {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::Accepted;
                                                        job.score += case.score;
                                                    } else if temp[0] == "Wrong Answer".to_string()
                                                    {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::WrongAnswer;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result = JobResult::WrongAnswer;
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    else {//输出信息不符合要求
                                                        job.cases[case_cnt].result =
                                                            CaseResult::SpjError;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result =
                                                                    JobResult::SpjError;
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    job.cases[case_cnt].info = temp[1].clone();
                                                }
                                            }
                                        }
                                        ProblemType::DynamicRanking => {
                                            let mut ratio = 0.0;
                                            if let Some(misc) = target_problem.misc.clone() {
                                                if let Some(r) = misc.dynamic_ranking_ratio.clone()
                                                {
                                                    ratio = r;
                                                }
                                            }
                                            let origin_file: String =
                                                std::fs::read_to_string(case.answer_file.clone())
                                                    .unwrap()
                                                    .trim()
                                                    .to_string();
                                            let origin: Vec<String> = origin_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let temp_file: String = std::fs::read_to_string(
                                                output_path.as_os_str().to_str().unwrap(),
                                            )
                                            .unwrap()
                                            .trim()
                                            .to_string();
                                            let temp: Vec<String> = temp_file
                                                .lines()
                                                .into_iter()
                                                .map(move |str| str.to_string().trim().to_string())
                                                .collect();
                                            let mut flag = true;
                                            if origin.len() != temp.len() {
                                                job.cases[case_cnt].result =
                                                    CaseResult::WrongAnswer;
                                                match job.result {
                                                    JobResult::Running => {
                                                        job.result = JobResult::WrongAnswer;
                                                    }
                                                    _ => {}
                                                }
                                                flag = false;
                                            } else {
                                                //逐行比对
                                                let mut i = 0;
                                                while i < origin.len() {
                                                    if origin[i] != temp[i] {
                                                        job.cases[case_cnt].result =
                                                            CaseResult::WrongAnswer;
                                                        match job.result {
                                                            JobResult::Running => {
                                                                job.result = JobResult::WrongAnswer;
                                                            }
                                                            _ => {}
                                                        }
                                                        flag = false;
                                                        break;
                                                    }
                                                    i += 1;
                                                }
                                            }
                                            if flag {
                                                //测试点通过
                                                job.cases[case_cnt].result = CaseResult::Accepted;
                                                job.score += case.score * ratio;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if !flag_pack {
                            //没有打包
                            let mut case_cnt = 0;
                            for case in &target_problem.cases {
                                case_cnt += 1;
                                let output_path = dir.path().join("test.out");
                                let out_file = File::create(&output_path).unwrap();
                                let in_file = File::open(case.input_file.clone()).unwrap();
                                let mut tle = false; //是否超时判断
                                let now = Instant::now();
                                let mut child = Command::new(format!("{}", exe_path_str))
                                    .stdin(Stdio::from(in_file))
                                    .stdout(Stdio::from(out_file))
                                    .stderr(Stdio::null())
                                    .spawn()
                                    .unwrap();
                                match child
                                    .wait_timeout(Duration::from_millis(
                                        (case.time_limit / 1000 + 500) as u64,
                                    ))
                                    .unwrap()
                                {
                                    Some(_status) => {}
                                    None => {
                                        tle = true;
                                        child.kill().unwrap();
                                    }
                                }
                                let new_now = Instant::now();
                                if tle {
                                    //超时！
                                    if let Some(dur) = new_now.checked_duration_since(now) {
                                        job.cases[case_cnt].time =
                                            (dur.as_secs_f64() * 1000000.0) as i32;
                                    }
                                    job.cases[case_cnt].result = CaseResult::TimeLimtExceeded;
                                    match job.result {
                                        JobResult::Running => {
                                            job.result = JobResult::TimeLimitExceeded;
                                        }
                                        _ => {}
                                    }
                                    continue;
                                }
                                let out_file = File::create(&output_path).unwrap();
                                let in_file = File::open(case.input_file.clone()).unwrap();
                                let now = Instant::now();
                                let status = Command::new(format!("{}", exe_path_str))
                                    .stdin(Stdio::from(in_file))
                                    .stdout(Stdio::from(out_file))
                                    .stderr(Stdio::null())
                                    .status()
                                    .unwrap();
                                let new_now = Instant::now();
                                if let Some(dur) = new_now.checked_duration_since(now) {
                                    job.cases[case_cnt].time =
                                        (dur.as_secs_f64() * 1000000.0) as i32;
                                }
                                if let Some(x) = status.code() {
                                    if x != 0 {
                                        job.cases[case_cnt].result = CaseResult::RuntimeError;
                                        match job.result {
                                            JobResult::Running => {
                                                job.result = JobResult::RuntimeError;
                                            }
                                            _ => {}
                                        }
                                        continue;
                                    }
                                }
                                match target_problem.ty {
                                    ProblemType::Standard => {
                                        //标准模式
                                        let origin_file: String =
                                            std::fs::read_to_string(case.answer_file.clone())
                                                .unwrap()
                                                .trim()
                                                .to_string();
                                        let origin: Vec<String> = origin_file
                                            .lines()
                                            .into_iter()
                                            .map(move |str| str.to_string().trim().to_string())
                                            .collect();
                                        let temp_file: String = std::fs::read_to_string(
                                            output_path.as_os_str().to_str().unwrap(),
                                        )
                                        .unwrap()
                                        .trim()
                                        .to_string();
                                        let temp: Vec<String> = temp_file
                                            .lines()
                                            .into_iter()
                                            .map(move |str| str.to_string().trim().to_string())
                                            .collect();
                                        let mut flag = true;
                                        if origin.len() != temp.len() {
                                            job.cases[case_cnt].result = CaseResult::WrongAnswer;
                                            match job.result {
                                                JobResult::Running => {
                                                    job.result = JobResult::WrongAnswer;
                                                }
                                                _ => {}
                                            }
                                            flag = false;
                                        } else {
                                            //逐行比对
                                            let mut i = 0;
                                            while i < origin.len() {
                                                if origin[i] != temp[i] {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::WrongAnswer;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result = JobResult::WrongAnswer;
                                                        }
                                                        _ => {}
                                                    }
                                                    flag = false;
                                                    break;
                                                }
                                                i += 1;
                                            }
                                        }
                                        if flag {
                                            //测试点通过
                                            job.cases[case_cnt].result = CaseResult::Accepted;
                                            job.score += case.score;
                                        }
                                    }
                                    ProblemType::Strict => {
                                        //严格模式，直接比较字符串
                                        let origin: String =
                                            std::fs::read_to_string(case.answer_file.clone())
                                                .unwrap();
                                        let temp: String = std::fs::read_to_string(
                                            output_path.as_os_str().to_str().unwrap(),
                                        )
                                        .unwrap();
                                        if origin != temp {
                                            job.cases[case_cnt].result = CaseResult::WrongAnswer;
                                            match job.result {
                                                JobResult::Running => {
                                                    job.result = JobResult::WrongAnswer;
                                                }
                                                _ => {}
                                            }
                                        } else {
                                            job.cases[case_cnt].result = CaseResult::Accepted;
                                            job.score += case.score;
                                        }
                                    }
                                    ProblemType::Spj => {
                                        //Special Judge
                                        if let Some(misc) = target_problem.misc.clone() {
                                            if let Some(command) = misc.special_judge.clone() {
                                                let program = command[0].clone();
                                                let mut args: Vec<String> = Vec::new();
                                                for arg in &command[1..] {
                                                    if arg == &"%OUTPUT%".to_string() {
                                                        args.push(
                                                            output_path
                                                                .as_os_str()
                                                                .to_str()
                                                                .unwrap()
                                                                .to_string()
                                                                .clone(),
                                                        );
                                                    } else if arg == &"%ANSWER%".to_string() {
                                                        args.push(case.answer_file.clone());
                                                    } else {
                                                        args.push(arg.clone());
                                                    }
                                                }
                                                let feedback =
                                                    File::create("src/feedback.out").unwrap();
                                                let _status = Command::new(program)
                                                    .args(args)
                                                    .stdout(Stdio::from(feedback))
                                                    .status()
                                                    .unwrap();
                                                let result: String =
                                                    std::fs::read_to_string("src/feedback.out")
                                                        .unwrap();
                                                let temp: Vec<String> = result
                                                    .lines()
                                                    .into_iter()
                                                    .map(move |str| {
                                                        str.to_string().trim().to_string()
                                                    })
                                                    .collect();
                                                if temp[0] == "Accepted".to_string() {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::Accepted;
                                                    job.score += case.score;
                                                } else if temp[0] == "Wrong Answer".to_string() {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::WrongAnswer;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result = JobResult::WrongAnswer;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                else {//输出信息不符合要求
                                                    job.cases[case_cnt].result =
                                                        CaseResult::SpjError;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result =
                                                                JobResult::SpjError;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                job.cases[case_cnt].info = temp[1].clone();
                                            }
                                        }
                                    }
                                    ProblemType::DynamicRanking => {
                                        let mut ratio = 0.0;
                                        if let Some(misc) = target_problem.misc.clone() {
                                            if let Some(r) = misc.dynamic_ranking_ratio.clone() {
                                                ratio = r;
                                            }
                                        }
                                        let origin_file: String =
                                            std::fs::read_to_string(case.answer_file.clone())
                                                .unwrap()
                                                .trim()
                                                .to_string();
                                        let origin: Vec<String> = origin_file
                                            .lines()
                                            .into_iter()
                                            .map(move |str| str.to_string().trim().to_string())
                                            .collect();
                                        let temp_file: String = std::fs::read_to_string(
                                            output_path.as_os_str().to_str().unwrap(),
                                        )
                                        .unwrap()
                                        .trim()
                                        .to_string();
                                        let temp: Vec<String> = temp_file
                                            .lines()
                                            .into_iter()
                                            .map(move |str| str.to_string().trim().to_string())
                                            .collect();
                                        let mut flag = true;
                                        if origin.len() != temp.len() {
                                            job.cases[case_cnt].result = CaseResult::WrongAnswer;
                                            match job.result {
                                                JobResult::Running => {
                                                    job.result = JobResult::WrongAnswer;
                                                }
                                                _ => {}
                                            }
                                            flag = false;
                                        } else {
                                            //逐行比对
                                            let mut i = 0;
                                            while i < origin.len() {
                                                if origin[i] != temp[i] {
                                                    job.cases[case_cnt].result =
                                                        CaseResult::WrongAnswer;
                                                    match job.result {
                                                        JobResult::Running => {
                                                            job.result = JobResult::WrongAnswer;
                                                        }
                                                        _ => {}
                                                    }
                                                    flag = false;
                                                    break;
                                                }
                                                i += 1;
                                            }
                                        }
                                        if flag {
                                            //测试点通过
                                            job.cases[case_cnt].result = CaseResult::Accepted;
                                            job.score += case.score * ratio;
                                        }
                                    }
                                }
                            }
                        }
                        //全部测试完成
                        job.state = State::Finished;
                        match job.result {
                            JobResult::Running => {
                                job.result = JobResult::Accepted;
                            }
                            _ => {}
                        }
                        let mut lock = JOBLIST.lock().unwrap();
                        lock[id.abs() as usize] = job.clone();
                        let mut jobs: Vec<Job> = Vec::new();
                        let mut i = 0;
                        while i < lock.len() {
                            jobs.push(lock[i].clone());
                            i += 1;
                        }
                        drop(lock);
                        //写入文件持久化存储
                        let out = std::fs::File::create("src/jobs.json").unwrap();
                        serde_json::to_writer(out, &jobs).unwrap();
                        HttpResponse::Ok().json(job)
                    }
                }
            }
        }
    }
}

///Post one user
#[post("/users")]
async fn post_users(user: web::Json<User>) -> impl Responder {
    if let Some(id) = user.id {
        //有id字段
        let mut lock = USERLIST.lock().unwrap();
        let mut i = 0;
        let mut flag_find = false; //标志是否找到该词
        let mut flag_overlap = false; //标志是否重复
        let mut index = 0;
        while i < lock.len() {
            if let Some(curid) = lock[i].id {
                if id == curid {
                    flag_find = true; //已找到
                    index = i;
                } else {
                    if lock[i].name == user.name {
                        flag_overlap = true; //重复
                    }
                }
            }
            i += 1;
        }
        if !flag_find {
            drop(lock);
            HttpResponse::NotFound().json(Error {
                code: 3,
                reason: "ERR_NOT_FOUND".to_string(),
                message: format!("User {} not found", id),
            })
        } else if flag_overlap {
            drop(lock);
            HttpResponse::BadRequest().json(Error {
                code: 1,
                reason: "ERR_INVALID_ARGUMENT".to_string(),
                message: format!("User name '{}' already exists", user.name.clone()),
            })
        } else {
            //修改
            lock[index].name = user.name.clone();
            let user = lock[index].clone();
            let mut users: Vec<User> = Vec::new();
            let mut i = 0;
            while i < lock.len() {
                users.push(lock[i].clone());
                i += 1;
            }
            drop(lock);
            //写入文件持久化存储
            let out = std::fs::File::create("src/users.json").unwrap();
            serde_json::to_writer(out, &users).unwrap();
            HttpResponse::Ok().json(user)
        }
    } else {
        //未指定id字段
        let mut lock = USERLIST.lock().unwrap();
        let id = lock.len();
        let mut flag_overlap = false;
        let mut i = 0;
        while i < lock.len() {
            if lock[i].name == user.name {
                flag_overlap = true;
                break;
            }
            i += 1;
        }
        if flag_overlap {
            drop(lock);
            HttpResponse::BadRequest().json(Error {
                code: 1,
                reason: "ERR_INVALID_ARGUMENT".to_string(),
                message: format!("User name '{}' already exists", user.name.clone()),
            })
        } else {
            lock.push(User {
                id: Some(id as i32),
                name: user.name.clone(),
            });
            let mut users: Vec<User> = Vec::new();
            let mut i = 0;
            while i < lock.len() {
                users.push(lock[i].clone());
                i += 1;
            }
            drop(lock);
            //写入文件持久化存储
            let out = std::fs::File::create("src/users.json").unwrap();
            serde_json::to_writer(out, &users).unwrap();
            HttpResponse::Ok().json(User {
                id: Some(id as i32),
                name: user.name.clone(),
            })
        }
    }
}

///Get the user list
#[get("/users")]
async fn get_users() -> impl Responder {
    let lock = USERLIST.lock().unwrap();
    let mut users: Vec<User> = Vec::new();
    let mut i = 0;
    while i < lock.len() {
        users.push(lock[i].clone());
        i += 1;
    }
    drop(lock);
    HttpResponse::Ok().json(users)
}

///Get the whole ranklist
#[get("/contests/0/ranklist")]
async fn get_ranklist(info: web::Query<ContestRules>, config: web::Data<Config>) -> impl Responder {
    //搜索所有Jobs
    let lock = JOBLIST.lock().unwrap();
    let mut jobs: Vec<Job> = Vec::new();
    let mut i = 0;
    while i < lock.len() {
        jobs.push(lock[i].clone());
        i += 1;
    }
    drop(lock);
    //首先对所有具有dynamic_ranking标志的题目计算分数
    for target_problem in config.problems.clone() {
        if let Some(misc) = target_problem.misc {
            if let Some(mut ratio) = misc.dynamic_ranking_ratio {
                //寻找到该题目为dynamic_ranking，此后开始遍历jobs
                ratio = 1.0 - ratio;
                let case_num = target_problem.cases.len();
                let mut min_times: Vec<i32> = Vec::new();
                for _i in 1..=case_num {
                    min_times.push(20000000);
                } //最小时间数列初始化
                let mut target_job_list: Vec<usize> = Vec::new();
                for job in &jobs {
                    if job.submission.problem_id == target_problem.id {
                        //提交的该题目id为目标题目
                        target_job_list.push(job.id as usize);
                        for i in 1..=case_num {
                            min_times[i - 1] = if job.cases[i].time < min_times[i - 1] {
                                job.cases[i].time
                            } else {
                                min_times[i - 1]
                            };
                        }
                    }
                }
                for index in target_job_list {
                    if jobs[index].score == 100.0 * ratio {
                        //只有拿到所有基础分数后才计算竞争得分
                        for i in 1..=case_num {
                            jobs[index].score += target_problem.cases[i - 1].score
                                * (1.0 - ratio)
                                * min_times[i - 1] as f64
                                / jobs[index].cases[i].time as f64;
                        }
                    }
                }
            } else {
                continue;
            }
        } else {
            continue;
        }
    }
    let mut userinfo: Vec<UserRanking> = Vec::new();
    let mut i = 0;
    let lock = USERLIST.lock().unwrap();
    let user_cnt = lock.len();
    //依照user_id向userinfo內加入成员
    while i < user_cnt {
        if let Some(id) = lock[i].id {
            userinfo.push(UserRanking {
                user_id: id,
                user: lock[i].clone(),
                scores: Vec::new(),
                time: Vec::new(),
                count: 0,
                tot_score: 0.0,
                latest: Utc.ymd(1970, 1, 1).and_hms(0, 0, 0),
            })
        }
        //按照problem_id向scores和time中增添元素
        let problem_cnt = config.problems.len();
        let mut j = 0;
        while j < problem_cnt {
            userinfo[i].scores.push(0.0);
            userinfo[i].time.push(Utc.ymd(1970, 1, 1).and_hms(0, 0, 0));
            j += 1;
        }
        i += 1;
    }
    drop(lock);
    //遍历所有Jobs
    for job in &jobs {
        let problem_id = job.submission.problem_id as usize;
        let user_id: usize = job.submission.user_id as usize;
        userinfo[user_id].count += 1;
        match info.scoring_rule {
            None | Some(ScoringRule::Latest) => {
                let time = Utc
                    .datetime_from_str(&job.updated_time.clone()[..], "%Y-%m-%dT%H:%M:%S%.3fZ")
                    .unwrap();
                if time > userinfo[user_id].time[problem_id] {
                    userinfo[user_id].tot_score -= userinfo[user_id].scores[problem_id];
                    userinfo[user_id].tot_score += job.score;
                    userinfo[user_id].scores[problem_id] = job.score;
                    userinfo[user_id].time[problem_id] = time.clone();
                    if time > userinfo[user_id].latest {
                        userinfo[user_id].latest = time;
                    }
                }
            }
            Some(ScoringRule::Highest) => {
                let time = Utc
                    .datetime_from_str(&job.updated_time.clone()[..], "%Y-%m-%dT%H:%M:%S%.3fZ")
                    .unwrap();
                if job.score > userinfo[user_id].scores[problem_id] {
                    println!("{} {}", job.score, problem_id);
                    userinfo[user_id].tot_score -= userinfo[user_id].scores[problem_id];
                    userinfo[user_id].tot_score += job.score;
                    userinfo[user_id].scores[problem_id] = job.score;
                    userinfo[user_id].time[problem_id] = time;
                    if time > userinfo[user_id].latest {
                        userinfo[user_id].latest = time;
                    }
                } else if job.score == userinfo[user_id].scores[problem_id] {
                    if time < userinfo[user_id].time[problem_id] {
                        userinfo[user_id].scores[problem_id] = job.score;
                        userinfo[user_id].time[problem_id] = time;
                        if time > userinfo[user_id].latest {
                            userinfo[user_id].latest = time;
                        }
                    }
                }
            }
        }
    }
    //排序规则
    let mut userincontest: Vec<UserinContest> = Vec::new();
    match info.tie_breaker {
        Some(TieBreaker::SubmissionCount) => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                let mut target_cnt = userinfo[0].count;
                let mut tie_flag = false;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                        target_cnt = userinfo[i].count;
                    } else if (userinfo[i].tot_score * 1000.0) as i64
                        == (target_score * 1000.0) as i64
                    {
                        if userinfo[i].count < target_cnt {
                            target = i;
                            target_score = userinfo[i].tot_score;
                            target_cnt = userinfo[i].count;
                        } else if userinfo[i].count == target_cnt {
                            tie_flag = true; //出现平手
                        }
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                if !tie_flag {
                    rank = userincontest.len() as i32 + 1;
                }
                userinfo.remove(target);
            }
        }
        Some(TieBreaker::SubmissionTime) => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                let mut target_time = userinfo[0].latest;
                let mut tie_flag = false;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                        target_time = userinfo[i].latest;
                    } else if (userinfo[i].tot_score * 1000.0) as i64
                        == (target_score * 1000.0) as i64
                    {
                        if userinfo[i].latest < target_time {
                            target = i;
                            target_score = userinfo[i].tot_score;
                            target_time = userinfo[i].latest;
                        } else if userinfo[i].latest == target_time {
                            tie_flag = true; //出现平手
                        }
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                if !tie_flag {
                    rank = userincontest.len() as i32 + 1;
                }
                userinfo.remove(target);
            }
        }
        Some(TieBreaker::UserId) => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                rank += 1;
                userinfo.remove(target);
            }
        }
        None => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                let mut tie_flag = false;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                    } else if (userinfo[i].tot_score * 1000.0) as i64
                        == (target_score * 1000.0) as i64
                    {
                        tie_flag = true;
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                if !tie_flag {
                    rank = userincontest.len() as i32 + 1;
                }
                userinfo.remove(target);
            }
        }
    }
    HttpResponse::Ok().json(userincontest)
}

///Post one contest
#[post("/contests")]
async fn post_contest(contest: web::Json<Contest>, config: web::Data<Config>) -> impl Responder {
    let mut contests: Vec<Contest> = Vec::new();
    let lock = CONTESTLIST.lock().unwrap();
    let mut i = 0;
    while i < lock.len() {
        contests.push(lock[i].clone());
        i += 1;
    }
    drop(lock);
    if let Some(id) = contest.id {
        if id as usize > contests.len() {
            HttpResponse::NotFound().json(Error {
                code: 3,
                reason: "ERR_NOT_FOUND".to_string(),
                message: format!("Contest {} not found", id),
            })
        } else {
            //contests的第id-1项为目标比赛
            let contest_new = Contest {
                id: contest.id,
                name: contest.name.clone(),
                from: contest.from.clone(),
                to: contest.to.clone(),
                problem_ids: contest.problem_ids.clone(),
                user_ids: contest.user_ids.clone(),
                submission_limit: contest.submission_limit,
            };
            let contestinfo_new = ContestInfo {
                id: contest.id,
                name: contest.name.clone(),
                from: contest.from.clone(),
                to: contest.to.clone(),
                problem_ids: contest.problem_ids.clone(),
                user_ids: contest.user_ids.clone(),
                submission_limit: contest.submission_limit,
                submission_time: 0,
            };
            //检查problems
            let mut flag_problem = true;
            for problem_id in contest_new.problem_ids.clone() {
                let mut flag = false;
                for problem in &config.problems {
                    if problem.id == problem_id {
                        flag = true;
                        break;
                    }
                }
                if !flag {
                    flag_problem = false;
                    break;
                }
            }
            //检查users
            let lock_user = USERLIST.lock().unwrap();
            let mut users: Vec<User> = Vec::new();
            let mut i = 0;
            while i < lock_user.len() {
                users.push(lock_user[i].clone());
                i += 1;
            }
            drop(lock_user);
            let mut flag_user = true;
            for user_id in contest_new.user_ids.clone() {
                let mut flag = false;
                for user in &users {
                    if user.id == Some(user_id) {
                        flag = true;
                        break;
                    }
                }
                if !flag {
                    flag_user = false;
                    break;
                }
            }
            if !flag_problem || !flag_user {
                HttpResponse::NotFound().json(Error {
                    code: 3,
                    reason: "ERR_NOT_FOUND".to_string(),
                    message: "Problem or user not exist".to_string(),
                })
            } else {
                let mut lock = CONTESTLIST.lock().unwrap();
                lock[(id - 1) as usize] = contest_new.clone();
                let mut contests: Vec<Contest> = Vec::new();
                let mut i = 0;
                while i < lock.len() {
                    contests.push(lock[i].clone());
                    i += 1;
                }
                drop(lock);
                //写入文件持久化存储
                let out = std::fs::File::create("src/contests.json").unwrap();
                serde_json::to_writer(out, &contests).unwrap();
                let mut lock = CONTESTINFOLIST.lock().unwrap();
                lock[(id - 1) as usize] = contestinfo_new.clone();
                drop(lock);
                HttpResponse::Ok().json(contest_new)
            }
        }
    } else {
        //未指定id
        let lock = CONTESTLIST.lock().unwrap();
        let contest_new = Contest {
            id: Some(lock.len() as i32 + 1),
            name: contest.name.clone(),
            from: contest.from.clone(),
            to: contest.to.clone(),
            problem_ids: contest.problem_ids.clone(),
            user_ids: contest.user_ids.clone(),
            submission_limit: contest.submission_limit,
        };
        let contestinfo_new = ContestInfo {
            id: Some(lock.len() as i32 + 1),
            name: contest.name.clone(),
            from: contest.from.clone(),
            to: contest.to.clone(),
            problem_ids: contest.problem_ids.clone(),
            user_ids: contest.user_ids.clone(),
            submission_limit: contest.submission_limit,
            submission_time: contest.submission_limit,
        };
        drop(lock);
        //检查problems
        let mut flag_problem = true;
        for problem_id in contest_new.problem_ids.clone() {
            let mut flag = false;
            for problem in &config.problems {
                if problem.id == problem_id {
                    flag = true;
                    break;
                }
            }
            if !flag {
                flag_problem = false;
                break;
            }
        }
        //检查users
        let lock_user = USERLIST.lock().unwrap();
        let mut users: Vec<User> = Vec::new();
        let mut i = 0;
        while i < lock_user.len() {
            users.push(lock_user[i].clone());
            i += 1;
        }
        drop(lock_user);
        let mut flag_user = true;
        for user_id in contest_new.user_ids.clone() {
            let mut flag = false;
            for user in &users {
                if user.id == Some(user_id) {
                    flag = true;
                    break;
                }
            }
            if !flag {
                flag_user = false;
                break;
            }
        }
        if !flag_problem || !flag_user {
            HttpResponse::NotFound().json(Error {
                code: 3,
                reason: "ERR_NOT_FOUND".to_string(),
                message: "Problem or user not exist".to_string(),
            })
        } else {
            let mut lock = CONTESTLIST.lock().unwrap();
            lock.push(contest_new.clone());
            let mut contests: Vec<Contest> = Vec::new();
            let mut i = 0;
            while i < lock.len() {
                contests.push(lock[i].clone());
                i += 1;
            }
            drop(lock);
            //写入文件持久化存储
            let out = std::fs::File::create("src/contests.json").unwrap();
            serde_json::to_writer(out, &contests).unwrap();
            let mut lock = CONTESTINFOLIST.lock().unwrap();
            lock.push(contestinfo_new.clone());
            drop(lock);
            HttpResponse::Ok().json(contest_new)
        }
    }
}

///Get the whole contest list
#[get("/contests")]
async fn get_contest() -> impl Responder {
    let lock = CONTESTLIST.lock().unwrap();
    let mut contests: Vec<Contest> = Vec::new();
    let mut i = 0;
    while i < lock.len() {
        contests.push(lock[i].clone());
        i += 1;
    }
    drop(lock);
    HttpResponse::Ok().json(contests)
}

///Get the contest by its contestid
#[get("/contests/{contestid}")]
async fn get_contest_id(contestid: web::Path<i32>) -> impl Responder {
    let lock = CONTESTLIST.lock().unwrap();
    let mut contests: Vec<Contest> = Vec::new();
    let mut i = 0;
    while i < lock.len() {
        contests.push(lock[i].clone());
        i += 1;
    }
    drop(lock);
    if contestid.abs() as usize > contests.len() {
        HttpResponse::NotFound().json(Error {
            code: 3,
            reason: "ERR_NOT_FOUND".to_string(),
            message: format!("Contest {} not found", contestid.abs()),
        })
    } else {
        HttpResponse::Ok().json(contests[contestid.abs() as usize - 1].clone())
    }
}

///Get the ranklist of one contest by its contestid
#[get("/contests/{contestid}/ranklist")]
async fn get_ranklist_id(
    contestid: web::Path<i32>,
    info: web::Query<ContestRules>,
    config: web::Data<Config>,
) -> impl Responder {
    //提取出该contest的info
    let lock = CONTESTINFOLIST.lock().unwrap();
    let target_contest = lock[contestid.abs() as usize - 1].clone();
    drop(lock);
    //搜索所有该contestid內的Jobs
    let lock = JOBLIST.lock().unwrap();
    let mut jobs: Vec<Job> = Vec::new();
    let mut i = 0;
    while i < lock.len() {
        if lock[i].submission.contest_id == contestid.abs() {
            jobs.push(lock[i].clone());
        }
        i += 1;
    }
    drop(lock);
    //首先对所有具有dynamic_ranking标志的题目计算分数
    for target_problem in config.problems.clone() {
        if let Some(misc) = target_problem.misc {
            if let Some(mut ratio) = misc.dynamic_ranking_ratio {
                //寻找到该题目为dynamic_ranking，此后开始遍历jobs
                ratio = 1.0 - ratio;
                let case_num = target_problem.cases.len();
                let mut min_times: Vec<i32> = Vec::new();
                for _i in 1..=case_num {
                    min_times.push(20000000);
                } //最小时间数列初始化
                let mut target_job_list: Vec<usize> = Vec::new();
                for job in &jobs {
                    if job.submission.problem_id == target_problem.id {
                        //提交的该题目id为目标题目
                        target_job_list.push(job.id as usize);
                        for i in 1..=case_num {
                            min_times[i - 1] = if job.cases[i].time < min_times[i - 1] {
                                job.cases[i].time
                            } else {
                                min_times[i - 1]
                            };
                        }
                    }
                }
                for index in target_job_list {
                    if jobs[index].score == 100.0 * ratio {
                        //只有拿到所有基础分数后才计算竞争得分
                        for i in 1..=case_num {
                            jobs[index].score += target_problem.cases[i - 1].score
                                * (1.0 - ratio)
                                * min_times[i - 1] as f64
                                / jobs[index].cases[i].time as f64;
                        }
                    }
                }
            } else {
                continue;
            }
        } else {
            continue;
        }
    }
    let mut userinfo: Vec<UserRanking> = Vec::new();
    let mut i = 0;
    let lock = USERLIST.lock().unwrap();
    let user_cnt = lock.len();
    //依照user_id向userinfo內加入该比赛内的成员
    while i < user_cnt {
        if let Some(id) = lock[i].id {
            if target_contest.user_ids.contains(&id) {
                userinfo.push(UserRanking {
                    user_id: id,
                    user: lock[i].clone(),
                    scores: Vec::new(),
                    time: Vec::new(),
                    count: 0,
                    tot_score: 0.0,
                    latest: Utc.ymd(1970, 1, 1).and_hms(0, 0, 0),
                });
                //按照problem_id向scores和time中增添元素
                let problem_cnt = config.problems.len();
                let mut j = 0;
                while j < problem_cnt {
                    if target_contest.problem_ids.contains(&(j as i32)) {
                        let len = userinfo.len();
                        userinfo[len - 1].scores.push(0.0);
                        userinfo[len - 1]
                            .time
                            .push(Utc.ymd(1970, 1, 1).and_hms(0, 0, 0));
                    }
                    //遍历所有jobs查找该用户改题目的提交
                    for job in &jobs {
                        if job.submission.contest_id == contestid.abs()
                            && job.submission.problem_id as usize == j
                            && job.submission.user_id == id
                        {
                            let problem_id = userinfo[userinfo.len() - 1].scores.len() - 1;
                            let user_id: usize = userinfo.len() - 1;
                            userinfo[user_id].count += 1;
                            match info.scoring_rule {
                                None | Some(ScoringRule::Latest) => {
                                    let time = Utc
                                        .datetime_from_str(
                                            &job.updated_time.clone()[..],
                                            "%Y-%m-%dT%H:%M:%S%.3fZ",
                                        )
                                        .unwrap();
                                    if time > userinfo[user_id].time[problem_id] {
                                        userinfo[user_id].tot_score -=
                                            userinfo[user_id].scores[problem_id];
                                        userinfo[user_id].tot_score += job.score;
                                        userinfo[user_id].scores[problem_id] = job.score;
                                        userinfo[user_id].time[problem_id] = time.clone();
                                        if time > userinfo[user_id].latest {
                                            userinfo[user_id].latest = time;
                                        }
                                    }
                                }
                                Some(ScoringRule::Highest) => {
                                    let time = Utc
                                        .datetime_from_str(
                                            &job.updated_time.clone()[..],
                                            "%Y-%m-%dT%H:%M:%S%.3fZ",
                                        )
                                        .unwrap();
                                    if job.score > userinfo[user_id].scores[problem_id] {
                                        println!("{} {}", job.score, problem_id);
                                        userinfo[user_id].tot_score -=
                                            userinfo[user_id].scores[problem_id];
                                        userinfo[user_id].tot_score += job.score;
                                        userinfo[user_id].scores[problem_id] = job.score;
                                        userinfo[user_id].time[problem_id] = time;
                                        if time > userinfo[user_id].latest {
                                            userinfo[user_id].latest = time;
                                        }
                                    } else if job.score == userinfo[user_id].scores[problem_id] {
                                        if time < userinfo[user_id].time[problem_id] {
                                            userinfo[user_id].scores[problem_id] = job.score;
                                            userinfo[user_id].time[problem_id] = time;
                                            if time > userinfo[user_id].latest {
                                                userinfo[user_id].latest = time;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }
    drop(lock);
    //排序规则
    let mut userincontest: Vec<UserinContest> = Vec::new();
    match info.tie_breaker {
        Some(TieBreaker::SubmissionCount) => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                let mut target_cnt = userinfo[0].count;
                let mut tie_flag = false;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                        target_cnt = userinfo[i].count;
                    } else if (userinfo[i].tot_score * 1000.0) as i64
                        == (target_score * 1000.0) as i64
                    {
                        if userinfo[i].count < target_cnt {
                            target = i;
                            target_score = userinfo[i].tot_score;
                            target_cnt = userinfo[i].count;
                        } else if userinfo[i].count == target_cnt {
                            tie_flag = true; //出现平手
                        }
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                if !tie_flag {
                    rank = userincontest.len() as i32 + 1;
                }
                userinfo.remove(target);
            }
        }
        Some(TieBreaker::SubmissionTime) => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                let mut target_time = userinfo[0].latest;
                let mut tie_flag = false;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                        target_time = userinfo[i].latest;
                    } else if (userinfo[i].tot_score * 1000.0) as i64
                        == (target_score * 1000.0) as i64
                    {
                        if userinfo[i].latest < target_time {
                            target = i;
                            target_score = userinfo[i].tot_score;
                            target_time = userinfo[i].latest;
                        } else if userinfo[i].latest == target_time {
                            tie_flag = true; //出现平手
                        }
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                if !tie_flag {
                    rank = userincontest.len() as i32 + 1;
                }
                userinfo.remove(target);
            }
        }
        Some(TieBreaker::UserId) => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                rank += 1;
                userinfo.remove(target);
            }
        }
        None => {
            let mut rank = 1;
            while !userinfo.is_empty() {
                let mut i = 1;
                let mut target = 0;
                let mut target_score = userinfo[0].tot_score;
                let mut tie_flag = false;
                while i < userinfo.len() {
                    if (userinfo[i].tot_score * 1000.0) as i64 > (target_score * 1000.0) as i64 {
                        target = i;
                        target_score = userinfo[i].tot_score;
                    } else if (userinfo[i].tot_score * 1000.0) as i64
                        == (target_score * 1000.0) as i64
                    {
                        tie_flag = true;
                    }
                    i += 1;
                }
                //导入userincontest
                userincontest.push(UserinContest {
                    user: userinfo[target].user.clone(),
                    rank,
                    scores: userinfo[target].scores.clone(),
                });
                if !tie_flag {
                    rank = userincontest.len() as i32 + 1;
                }
                userinfo.remove(target);
            }
        }
    }
    HttpResponse::Ok().json(userincontest)
}

// DO NOT REMOVE: used in automatic testing
#[post("/internal/exit")]
#[allow(unreachable_code)]
async fn exit() -> impl Responder {
    log::info!("Shutdown as requested");
    std::process::exit(0);
    format!("Exited")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    let data = fs::read_to_string(args.config).unwrap();
    let config: Config = serde_json::from_str(&data).unwrap();
    let mut lock = USERLIST.lock().unwrap();
    lock.push(User {
        id: Some(0),
        name: "root".to_string(),
    });
    drop(lock);
    //持久化存储的数据读取，包括jobs,contests,users
    if args.flushdata {
        fs::write("src/jobs.json", "[]".to_string()).unwrap();
    }
    let mut lock = JOBLIST.lock().unwrap();
    let str = fs::read_to_string("src/jobs.json").unwrap();
    let jobs: Vec<Job> = serde_json::from_str(&str[..]).unwrap();
    for job in jobs {
        lock.push(job.clone());
    }
    drop(lock);
    if args.flushdata {
        fs::write("src/users.json", "[]".to_string()).unwrap();
    }
    let mut lock = USERLIST.lock().unwrap();
    let str = fs::read_to_string("src/users.json").unwrap();
    let users: Vec<User> = serde_json::from_str(&str[..]).unwrap();
    for user in users {
        lock.push(user.clone());
    }
    drop(lock);
    if args.flushdata {
        fs::write("src/contests.json", "[]".to_string()).unwrap();
    }
    let mut lock = CONTESTLIST.lock().unwrap();
    let str = fs::read_to_string("src/contests.json").unwrap();
    let contests: Vec<Contest> = serde_json::from_str(&str[..]).unwrap();
    for contest in contests {
        lock.push(contest.clone());
    }
    drop(lock);
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
            .wrap(Logger::default())
            .route("/hello", web::get().to(|| async { "Hello World!" }))
            .service(greet)
            .service(post_jobs)
            .service(get_jobs)
            .service(get_job_id)
            .service(put_job_id)
            .service(post_users)
            .service(get_users)
            .service(get_ranklist)
            .service(post_contest)
            .service(get_contest)
            .service(get_contest_id)
            .service(get_ranklist_id)
            // DO NOT REMOVE: used in automatic testing
            .service(exit)
    })
    .bind(("127.0.0.1", 12345))?
    .run()
    .await
}
