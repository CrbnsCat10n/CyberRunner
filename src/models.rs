use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const LOCAL_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub const SIGN_SALT: &str = "TongjiSports";
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
pub const DEFAULT_TARGET: &str = "/msports/api/public/running/save";
pub const RUN_COUNT_TARGET: &str = "/msports/api/public/running/run/count";
pub const DEFAULT_REFERER: &str = "https://servicewechat.com/wx89b3d9ebb11efee3/29/page-frame.html";
pub const DEFAULT_BASE_URL: &str = "https://ty.tongji.edu.cn/msports";
pub const PRODUCTION_HOST: &str = "tiyu.tongji.edu.cn";
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 26_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.73(0x18004934) NetType/4G Language/zh_CN";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Venue {
    pub index: usize,
    pub venue_id: String,
    pub venue_name: String,
    pub campus_name: String,
    pub open: Option<bool>,
    pub polygon_lonlat: Vec<(f64, f64)>,
    pub raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub time: String,
    pub speed: f64,
    pub altitude: f64,
    pub attribute1: i32,
    pub dist: f64,
}

#[derive(Clone, Debug)]
pub struct ReplayConfig {
    pub login_name: String,
    pub semester_id: String,
    pub semester_name: String,
    pub sex: String,
    pub run_status: String,
    pub standard_pace: String,
    pub result_km: String,
    pub track_km: f64,
    pub duration_minutes: f64,
    pub start_time: NaiveDateTime,
    pub inset_m: f64,
    pub seed: u64,
    pub user_agent: Option<String>,
    pub authorization: Option<String>,
    pub referer: String,
    pub packet_seconds: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            login_name: String::new(),
            semester_id: "121".to_owned(),
            semester_name: "2025-2026学年第2学期".to_owned(),
            sex: "0".to_owned(),
            run_status: "0".to_owned(),
            standard_pace: "8.00".to_owned(),
            result_km: "2.00".to_owned(),
            track_km: 2.0,
            duration_minutes: 10.0,
            start_time: chrono::Local::now().naive_local(),
            inset_m: 15.0,
            seed: 20260601,
            user_agent: None,
            authorization: None,
            referer: DEFAULT_REFERER.to_owned(),
            packet_seconds: 60,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratedPacket {
    pub index: usize,
    pub total: usize,
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub scheduled_at: NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackPayload {
    pub center: [f64; 2],
    pub line: Vec<[f64; 2]>,
    pub point_count: usize,
    pub distance_km: f64,
    pub start_time: String,
    pub end_time: String,
}
