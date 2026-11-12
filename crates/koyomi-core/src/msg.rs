//! The message catalogue: every error and warning koyomi raises has an ID
//! (`KOYOMI-E2101`), and its words live in `messages/en.toml` and `ja.toml`.
//!
//! **An ID is never removed and never reused.** A stored run reason or an old
//! log can still name it. A message nobody raises any more stays in the list
//! and in the catalogue. When a message's meaning changes or it splits, it
//! gets a new ID.
//!
//! The number is a letter (`E` error, `W` warning) and four digits: the first
//! two are the area (see `[area]` in the catalogue), the last two count within
//! it. The same catalogue makes the troubleshooting page (`docs/troubleshooting.md`,
//! written by `cargo run -p koyomi-core --example make-messages`) and the
//! window's card.
//!
//! A sentence that is not an error or a warning (a completion notice, a progress
//! line, a heading, a status word) is a **note**: `[note]` in the same catalogue,
//! flat keys, no ID and no `when`, `cause` or `action`. [`note`] gives it in the
//! process's language. A note is never shown with an ID, so it is never coded
//! ([`is_coded`] is false for it) and the troubleshooting page does not list it.

use std::sync::{LazyLock, RwLock};

use ito_i18n::Table;

const TABLES: &[(&str, &str)] = &[("en", include_str!("../messages/en.toml")), ("ja", include_str!("../messages/ja.toml"))];

/// Words a Japanese message must not use. A later spec can add to the list.
#[cfg(test)]
const RETIRED_WORDS: &[&str] = &[
    "つなぎ先",
    "向こう",
    "流す",
    "つながらない",
    "見る。",
    "確かめて",
    "秘密",
    "窓",
    "暦",
    "もうある",
    "時間切れ",
    "ほかで変わって",
    "読めない",
    "書けない",
    "が要る",
    "知らない欄",
    "部品",
    "だめ",
];

/// A message's ID, such as `E2101`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Id(&'static str);

impl Id {
    /// `KOYOMI-E2101`.
    pub fn code(self) -> String {
        format!("KOYOMI-{}", self.0)
    }

    /// Whether `s` starts with this ID's code (`KOYOMI-E2101`).
    pub fn is_in(self, s: &str) -> bool {
        s.starts_with(&self.code())
    }

    /// `E2101`.
    pub fn number(self) -> &'static str {
        self.0
    }

    /// Whether the letter is `W`.
    pub fn is_warning(self) -> bool {
        self.0.starts_with('W')
    }

    /// The first two digits: `21`.
    pub fn area(self) -> &'static str {
        &self.0[1..3]
    }
}

macro_rules! ids {
    ($($name:ident = $code:literal),* $(,)?) => {
        $(pub const $name: Id = Id($code);)*
        /// Every ID there is.
        pub const ALL: &[Id] = &[$($name),*];
    }
}

ids! {
    CONNECT_FAILED = "E2101",
    MAY_HAVE_ARRIVED = "E2102",
    CERT_MISMATCH = "E2103",
    TIMED_OUT = "E2104",
    TOKEN_SECRET_MISSING = "E2105",
    TOKEN_SECRET_UNREADABLE = "E2106",
    TOKEN_REFUSED = "E2107",
    PROTOCOL_MISMATCH = "E2108",
    REPLY_UNREADABLE = "E2109",
    ENGINE_TOO_OLD = "E2110",
    TLS_FAILED = "E2111",
    ADDRESS_UNREADABLE = "E2112",
    UNEXPECTED_STATUS = "E2113",
    NOT_ADMITTED = "E2114",
    NO_REMOTE = "E2115",
    UNKNOWN_REMOTE = "E2116",
    LINK_SWITCHING = "E2117",
    TOKEN_STORE_FAILED = "E2120",
    CONFIG_WRITE_FAILED = "E2121",
    CONFIG_WRITE_FAILED_TOKEN_LEFT = "E2122",
    CONNECTION_IN_USE = "E2123",
    URL_NOT_CHANGED = "E2124",
    REMOTE_CHANGED_DURING_CHECK = "E2125",
    NAME_EMPTY = "E2130",
    NAME_TAKEN = "E2131",
    NAME_UNUSABLE = "E2132",
    URL_BAD = "E2133",
    TOKEN_EMPTY = "E2134",
    SECRET_IN_USE = "E2135",
    SECRET_EXISTS = "E2136",
    REMOTE_GONE = "E2137",
    NAMED_BY_TABLE = "E2138",
    REWRITE_MISMATCH = "E2139",
    PRIMARY_STANDBY_UNKNOWN = "W2140",
    NO_STANDBY_THERE = "W2141",
    HA_UNREADABLE = "W2142",
    SSH_CONNECT = "E2150",
    SSH_AUTH_REFUSED = "E2151",
    SSH_HOST_KEY_MISMATCH = "E2152",
    LINK_LOST = "E2153",
    LINK_TIMED_OUT = "E2154",
    WINRM_CONNECT = "E2160",
    WINRM_BASIC_REFUSED = "E2161",
    WINRM_NTLM_REFUSED = "E2162",
    WINRM_CERT_MISMATCH = "E2163",
    WINRM_TLS = "E2164",
    WINRM_FAULT = "E2165",
    MAIL_CONNECT = "E2170",
    MAIL_TLS = "E2171",
    MAIL_LOGIN = "E2172",
    MAIL_NO_FOLDER = "E2173",
    MAIL_SECRET_MISSING = "E2174",
    MAIL_SECRET_UNREADABLE = "E2175",
    MAIL_UNREADABLE = "E2176",
    ROLE_TOO_LOW = "E2201",
    ROLE_TOO_LOW_ON_NET = "E2202",
    USER_UNKNOWN = "E2203",
    TOKEN_REQUIRED = "E2204",
    DOOR_NEEDS_TOKEN = "E2205",
    DOOR_REFUSED_NARROWED = "E2206",
    DOOR_NOT_ALLOWED = "E2207",
    NARROWED_ALL_NETS = "E2208",
    OUTSIDE_NETS = "E2209",
    HANDOVER_REFUSED = "E2210",
    SAVE_BLOCKED = "E3101",
    SAVE_CHANGED_ELSEWHERE = "E3102",
    CONFLICT_NEAR = "E3103",
    CONFLICT_FAR = "E3104",
    CREATED_ELSEWHERE = "E3105",
    DEF_INVALID_SAVE = "E3106",
    DEF_INVALID_STAGE = "E3107",
    STAGE_DATE_PAST = "E3108",
    STAGE_DATE_BAD = "E3109",
    STAGE_NO_LIVE = "E3110",
    STAGE_CHANGED_ELSEWHERE = "E3111",
    STAGE_SAME = "E3112",
    IMPORT_PROBLEMS = "E3113",
    EDITOR_PROBLEMS = "E3114",
    DEF_INVALID_SWITCH = "E3115",
    JOBNET_EXISTS = "E3150",
    JOBNET_NAME_EMPTY = "E3151",
    JOBNET_NAME_BAD = "E3152",
    JOBNET_ID_FORM = "E3153",
    UNSAVED_EDITS = "E3154",
    DELETE_BLOCKED = "E3155",
    FILE_WRITE_FAILED = "E3156",
    DIR_CREATE_FAILED = "E3157",
    FILE_READ_FAILED = "E3158",
    FILE_DELETE_FAILED = "E3159",
    FILE_ENCODING_UNKNOWN = "E3160",
    LIST_UNAVAILABLE = "E3161",
    CALENDAR_EXISTS = "E4101",
    CAL_NAME_EMPTY = "E4102",
    CAL_NAME_BAD = "E4103",
    CAL_MISSING = "E4104",
    CAL_CHANGED_ELSEWHERE = "E4105",
    CAL_UNKNOWN_FIELD = "E4106",
    CAL_TOML_SYNTAX = "E4107",
    CAL_FORMAT_MISSING = "E4108",
    CAL_FORMAT_UNSUPPORTED = "E4109",
    CAL_HOLIDAYS_VALUE = "E4110",
    CAL_DAY_VALUE = "E4111",
    CAL_ENTRY_SHAPE = "E4112",
    CAL_SAME_NAME = "E4113",
    CAL_USERS_LEFT = "W4114",
    TABLE_DATE_UNREADABLE = "E4150",
    TABLE_EMPTY = "E4151",
    TABLE_ENDING_SOON = "W4152",
    TABLE_STALE = "W4153",
    TABLE_PATH_EMPTY = "E4154",
    CFG_GROUP_NOT_ARRAY = "E1001",
    CFG_GROUP_TOO_MANY = "E1002",
    CFG_GROUP_UNKNOWN_KEY = "E1003",
    CFG_GROUP_NOT_STRING = "E1004",
    CFG_GROUP_NO_ON = "E1005",
    CFG_GROUP_BRACES = "E1006",
    CFG_GROUP_ADDRESS = "E1007",
    CFG_GROUP_MEMBER = "E1008",
    CFG_GROUP_MIXED = "E1009",
    CFG_GROUP_KEY_SCHEME = "E1010",
    CFG_GROUP_DUPLICATE = "E1011",
    CFG_NAME_NOT_PLAIN = "E1012",
    CFG_GROUP_MISSING = "E1013",
    CFG_GROUP_WRONG_KIND = "E1014",
    CFG_POLL_TARGET = "E1015",
    CFG_EVERY_UNREADABLE = "E1016",
    CFG_EVERY_RANGE = "E1017",
    CFG_MISSES_RANGE = "E1018",
    CFG_PING_URL = "E1019",
    CFG_REMOTE_NAME = "E1020",
    CFG_REMOTE_URL = "E1021",
    CFG_REMOTE_FINGERPRINT = "E1022",
    CFG_TOKEN_NOT_SECRET = "E1023",
    CFG_TOKEN_HASH_BAD = "E1071",
    CFG_USER_TOKEN_FORM = "E1072",
    CFG_HTTP_PORT = "E1024",
    CFG_TAILNET_PORT = "E1025",
    CFG_TAILNET_NO_USER = "E1026",
    CFG_USER_NAME = "E1027",
    CFG_DUPLICATE = "E1028",
    CFG_USER_FIELD = "E1029",
    CFG_USER_ADMIN_NARROWED = "E1030",
    CFG_TAILSCALE_FORM = "E1031",
    CFG_TAILSCALE_NEEDS_TAILNET = "E1032",
    CFG_UNKNOWN_KEY = "E1033",
    CFG_MISSING_KEY = "E1034",
    CFG_VALUE_UNKNOWN = "E1035",
    CFG_UNREADABLE = "E1036",
    CFG_LIMIT_TOO_LOW = "E1037",
    CFG_STANDBY_AND_PRIMARY = "E1038",
    CFG_LOCK_NAME = "E1039",
    CFG_LOCK_COUNT = "E1040",
    CFG_WORK_DIR = "E1041",
    CFG_VARS_NOT_STRING = "E1042",
    CFG_VARS_NESTED = "E1043",
    CFG_VARS_VALUE = "E1044",
    SERVICE_NOT_INSTALLED = "E1045",
    SERVICE_INSTALLED_ALREADY = "E1046",
    SERVICE_PASSWORD_EMPTY = "E1047",
    SERVICE_SIGNIN_ENGINE_RUNNING = "E1048",
    SERVICE_STOPPED_AT_START = "E1049",
    SERVICE_NO_ANSWER = "E1050",
    SERVICE_NOT_RUN_BY_WINDOWS = "E1051",
    SERVICE_UNSUPPORTED = "E1052",
    SERVICE_NEEDS_ADMIN = "E1053",
    SERVICE_NAME_BAD = "E1054",
    SERVICE_ACCOUNT_BAD = "E1055",
    SERVICE_LOGON_FAILED = "E1056",
    SERVICE_TIMEOUT = "E1057",
    SERVICE_NO_CONSOLE = "E1058",
    SERVICE_REFUSED = "E1059",
    EXE_IN_TEMP = "E1060",
    ENGINE_NOT_STARTED = "E1061",
    ENGINE_SERVICE_STOPPED_WINDOWS = "E1062",
    ENGINE_SERVICE_STOPPED_LINUX = "E1063",
    SERVICE_STATE_UNREADABLE = "E1064",
    ENGINE_SPAWN_FAILED = "E1065",
    KEYS_ADDRESS_MISSING = "E1066",
    ENGINE_NOT_RUNNING = "E1067",
    CHECK_NOTIFY_UNKNOWN = "W1068",
    CHECK_VAR_MISSING = "W1069",
    CHECK_GROUP_KEY_MISSING = "W1070",
    ROLE_NOT_KNOWN = "E2231",
    NETS_EMPTY = "E2232",
    ROLES_EMPTY = "E2233",
    NET_PATTERN_BAD = "E2234",
    ROLES_VALUE_BAD = "E2235",
    USER_NOT_IN_FILE = "E2236",
    USER_LAST_ADMIN = "E2237",
    USER_BASE_CHANGED = "E2238",
    USER_OWN_ADMIN = "E2239",
    USER_NO_FILE = "E2240",
    USER_EXISTS = "E2241",
    USER_TOKEN_FAILED = "E2242",
    USER_RESET_OWN = "E2243",
    USER_NO_HTTP = "E2244",
    USER_UNSURE = "E2245",
    USER_REPLY_BAD = "E2246",
    CONFIG_CHANGED = "E2247",
    CONFIG_SECRET = "E2248",
    CONFIG_CUTS_LINK = "E2249",
    PREFLIGHT_EXE_MISSING = "E3181",
    PREFLIGHT_DIR_MISSING = "E3182",
    PREFLIGHT_WATCH_MISSING = "E3183",
    PREFLIGHT_WORK_MISSING = "E3184",
    PREFLIGHT_DAY_ABSENT = "W3185",
    RETARGET_UNFOLLOWED = "E3186",
    DEF_FIELD_UNKNOWN = "E3301",
    DEF_KEY_DUPLICATE = "E3302",
    DEF_TOML_SYNTAX = "E3303",
    DEF_FORMAT_MISSING = "E3304",
    DEF_FORMAT_UNSUPPORTED = "E3305",
    DEF_CHOICE_BAD = "E3306",
    DEF_STRING_BAD = "E3307",
    DEF_STRING_BAD_E = "E3308",
    DEF_VALUE_EMPTY = "E3309",
    DEF_TABLE_BAD = "E3310",
    DEF_ARRAY_BAD = "E3311",
    DEF_NAME_BAD = "E3312",
    DEF_ENV_RESERVED = "E3313",
    DEF_VALUE_STRING_BAD = "E3314",
    DEF_SECRET_NAME_EMPTY = "E3315",
    DEF_PASSWORD_NOT_SECRET = "E3316",
    DEF_TEMPLATE_BAD = "E3317",
    DEF_VALUE_UNREADABLE = "E3318",
    DEF_INT_RANGE = "E3319",
    DEF_PRIORITY_BAD = "E3320",
    DEF_WAKE_BAD = "E3321",
    DEF_LATE_SLOW_BAD = "E3322",
    DEF_REQUIRE_BAD = "E3323",
    DEF_NAMES_DUPLICATE = "E3324",
    DEF_CALENDAR_NAME_BAD = "E3325",
    DEF_LOCK_SHAPE = "E3326",
    DEF_LOCK_NAME_BAD = "E3327",
    DEF_SUBKEY_MISSING = "E3328",
    DEF_SUBKEY_UNKNOWN = "E3329",
    DEF_LOCK_SHARED_BAD = "E3330",
    DEF_WHEN_KIND_MISSING = "E3331",
    DEF_WHEN_KEY_UNUSED = "E3332",
    DEF_ON_CLOSED_BAD = "E3333",
    DEF_JOBNET_REF_BAD = "E3334",
    DEF_WHEN_KEY_MISSING = "E3335",
    DEF_EITHER_REQUIRED = "E3336",
    DEF_MONTH_RULE_BAD = "E3337",
    DEF_ON_START_BAD = "E3338",
    DEF_IDLE_SHORT = "E3339",
    DEF_WAIT_UNTIL_ALONE = "E3340",
    DEF_PERIOD_ORDER = "E3341",
    DEF_PATH_NOT_ABSOLUTE = "E3342",
    DEF_LOG_PATH_BAD = "E3343",
    DEF_FILE_NAME_MISSING = "E3344",
    DEF_FILE_WILDCARD_DIR = "E3345",
    DEF_IDS_BAD = "E3346",
    DEF_SESSION_USER_BAD = "E3347",
    DEF_BUSINESS_DAY_CONFLICT = "E3348",
    DEF_BUSINESS_DAY_BAD = "E3349",
    DEF_SHIFT_NEEDS_DAY = "E3350",
    DEF_DAY_BAD = "E3351",
    DEF_TIME_ORDER = "E3352",
    DEF_CHILD_LOOP = "E3353",
    DEF_REGEX_BAD = "E3354",
    GROUP_FILE_UNKNOWN_KEY = "E3370",
    GROUP_FILE_SYNTAX = "E3371",
    GROUP_FILE_NO_FORMAT = "E3372",
    GROUP_FILE_FORMAT = "E3373",
    GROUP_FILE_CALENDAR = "E3374",
    DEF_NEEDS_CALENDAR = "E3375",
    CHILD_MISSING = "E3376",
    CHILD_INVALID = "E3377",
    BEFORE_MISSING = "E3378",
    BEFORE_LOOP = "E3379",
    DEF_NOT_UTF8 = "E3380",
    BLACKOUT_TIME_BAD = "E3381",
    BLACKOUT_ORDER = "E3382",
    QUIET_SPAN_BAD = "E3383",
    QUIET_SPAN_SAME = "E3384",
    DEADLINE_BAD = "E3385",
    ONLY_ON_STANDBY = "E3386",
    TIME_BAD = "E3387",
    WEEKDAY_BAD = "E3388",
    DATE_BAD = "E3389",
    DATETIME_BAD = "E3390",
    MONTH_DAY_BAD = "E3391",
    MAIL_SHAPE = "E3392",
    MAIL_NOT_TLS = "E3393",
    MAIL_PORT_BAD = "E3394",
    DEF_ONLY_FOR_KINDS = "E3401",
    DEF_JOB_KEY_REQUIRED = "E3402",
    DEF_JOB_KEY_REQUIRED_PLAIN = "E3403",
    DEF_LOOP_BOTH = "E3404",
    DEF_KEEP_DAYS_BAD = "E3405",
    DEF_ROWS_BAD = "E3406",
    DEF_EXPECT_BAD = "E3407",
    DEF_RUN_AS_USER_BAD = "E3408",
    DEF_RANGE_EMPTY = "E3409",
    DEF_RANGE_CODE_BAD = "E3410",
    DEF_RANGE_REVERSED = "E3411",
    DEF_LINE_BOTH = "E3412",
    DEF_JUDGE_EMPTY = "E3413",
    DEF_ID_MISSING = "E3414",
    DEF_ID_BAD = "E3415",
    DEF_ID_DUPLICATE = "E3416",
    DEF_RUN_SHELL_BOTH = "E3417",
    DEF_SHELL_NEEDS_SCRIPT = "E3418",
    DEF_SCRIPT_NEEDS_SHELL = "E3419",
    DEF_RUN_MISSING = "E3420",
    DEF_RUN_PROGRAM_EMPTY = "E3421",
    DEF_AFTER_ELEMENT_BAD = "E3422",
    DEF_BRANCH_AFTER_COUNT = "E3423",
    DEF_AFTER_UNKNOWN_ID = "E3424",
    DEF_CODE_LINE_NOT_BRANCH = "E3425",
    DEF_JOB_NO_BEFORE = "E3426",
    DEF_JOB_NO_NEXT = "E3427",
    DEF_AFTER_CYCLE = "E3428",
    DEF_POS_BAD = "E3429",
    DEF_ON_EMPTY = "E3430",
    WIN_JOB_ID_BAD = "E3431",
    WIN_HEADER_BAD = "E3432",
    WIN_STATUS_CODE_BAD = "E3433",
    WIN_JOB_GONE = "E3434",
    DURATION_ZERO = "E3470",
    DURATION_UNREADABLE = "E3471",
    TEMPLATE_NO_CALENDAR = "E3472",
    TEMPLATE_NO_DAY = "E3473",
    VAR_NAME_EMPTY = "E3474",
    VAR_NAME_BAD = "E3475",
    PARAM_MISSING = "E3476",
    VAR_MISSING = "E3477",
    PARAMS_NONE = "E3478",
    PARAM_UNKNOWN = "E3479",
    TOKEN_NO_NUMBER = "E3480",
    TOKEN_NO_UNIT = "E3481",
    TOKEN_UNIT_BAD = "E3482",
    TOKEN_FORMAT_BAD = "E3483",
    TEMPLATE_EXPAND = "E3484",
    STATE_NOT_UTF8 = "E3770",
    RETAIN_NAME_NOT_UTF8 = "E3771",
    RETAIN_RUN_ID_BAD = "E3772",
    DELETE_USED = "E3766",
    JOBNET_MISSING = "E3720",
    JOBNET_UNREADABLE = "E3721",
    NET_JOB_MISSING = "E3722",
    HOLD_ALREADY = "E3723",
    HOLD_JOB_ALREADY = "E3724",
    HOLD_NOT = "E3725",
    HOLD_JOB_NOT = "E3726",
    TIME_FORM_BAD = "E3727",
    DATE_FORM_BAD = "E3728",
    BUSINESS_DAY_FORM_BAD = "E3729",
    SHIFT_NOT_SHIFTED = "E3730",
    SHIFT_NO_SUCH_TIME = "E3731",
    SHIFT_BEFORE = "E3732",
    SHIFT_PAST = "E3733",
    SKIP_NOT = "E3734",
    SKIP_UNCONFIRMED = "E3735",
    SKIP_NO_NEXT = "E3736",
    PAUSE_ALREADY = "E3737",
    PAUSE_NOT = "E3738",
    BLOCKED_BY_WARNINGS = "E3739",
    RUN_INVALID = "E3740",
    RUN_UNCONFIRMED = "E3741",
    RUN_HELD = "E3742",
    APPROVE_BLOCKED = "E3743",
    APPROVE_INVALID = "E3744",
    APPROVE_CHANGED = "E3745",
    UNSTAGE_NONE = "E3746",
    SWITCH_FAILED = "W3747",
    SWITCH_NO_LIVE = "E3748",
    OS_NO_PWSH = "W3749",
    OS_RUN_AS = "W3750",
    OS_EVENTLOG = "W3751",
    CHILD_UNCONFIRMED = "E3356",
    WIN_WHEN_KIND_UNKNOWN = "E3357",
    WIN_DATETIME_BAD = "E3358",
    IMPORT_FAILED = "E3619",
    IMPORT_EMPTY = "E3620",
    IMPORT_STOPPED = "E3621",
    EXPORT_NOTHING = "E3622",
    VERSION_FORM_BAD = "E3624",
    VERSION_MISSING = "E3625",
    JOBNET_RUNNING = "E3756",
    RENAME_USED = "E3757",
    FILE_MOVE_FAILED = "E3758",
    REPLACE_FROM_EMPTY = "E3759",
    REPLACE_SAME = "E3760",
    REPLACE_UNREADABLE = "E3761",
    REPLACE_WRITE_FAILED = "E3762",
    HTTP_FORBIDDEN = "E3763",
    ENGINE_ALREADY_RUNNING = "E3764",
    ENGINE_LOCK_HELD = "E3765",
    HANDOVER_TOO_NEAR = "E3717",
    HANDOVER_NOT_STANDBY = "E3718",
    NO_STANDBY_OR_PRIMARY = "E3719",
    SETTINGS_DIR_UNKNOWN = "E3701",
    STATE_DIR_UNKNOWN = "E3702",
    CONFIG_LOAD_FAILED = "E3703",
    APPROVED_LOAD_FAILED = "E3704",
    HELD_LOAD_FAILED = "E3705",
    NEXT_LOAD_FAILED = "E3706",
    FILE_WRITABLE_BY = "W3707",
    TRAY_ALREADY_RUNNING = "E3708",
    TRAY_UNAVAILABLE = "E3709",
    ENGINE_HAS_TRAY = "E3710",
    PING_URL_PLAIN = "W3711",
    PING_SECRET_MISSING = "E3712",
    PING_SECRET_UNREADABLE = "E3713",
    IDLE_UNKNOWN = "W3714",
    LOG_WATCH_UNREADABLE = "E3715",
    ENGINE_WAS_DOWN = "W3716",
    TASK_XML_UNREADABLE = "E3501",
    TASK_NOT_SCHEDULER_XML = "E3502",
    TASK_NO_EXEC = "E3503",
    TASK_WAS_DISABLED = "W3504",
    TASK_USER_DROPPED = "W3505",
    TASK_NO_TIMES = "W3506",
    TASK_ACTION_DROPPED = "W3507",
    TASK_PERCENT = "W3508",
    TASK_TRIGGER_OFF = "W3509",
    TASK_TIME_UNREADABLE = "W3510",
    TASK_ONCE_NO_START = "W3511",
    TASK_LOGON_AS_START = "W3512",
    TASK_IDLE = "W3513",
    TASK_SESSION_STATE = "W3514",
    TASK_SESSION_USER = "W3515",
    TASK_TRIGGER_KIND = "W3516",
    TASK_REPEAT_KIND = "W3517",
    TASK_INTERVAL = "W3518",
    TASK_WEEK_NO_DAY = "W3519",
    TASK_MONTH_NO_DAY = "W3520",
    TASK_LAST_PARTIAL = "W3521",
    TASK_MONTH_DAY_BAD = "W3522",
    TASK_NTH_PARTIAL = "W3523",
    TASK_NTH_WEEK = "W3524",
    TASK_NTH_NONE = "W3525",
    TASK_CALENDAR_BAD = "W3526",
    TASK_REPEAT_OVERNIGHT = "W3527",
    TASK_REPEAT_ENDLESS = "W3528",
    TASK_REPEAT_SECONDS = "W3529",
    BUNDLE_TOML_BAD = "E3601",
    BUNDLE_FORMAT_MISSING = "E3602",
    BUNDLE_FORMAT_NEWER = "E3603",
    BUNDLE_JOBNET_ID = "E3604",
    BUNDLE_NAME_BAD = "E3605",
    BUNDLE_PICK_EMPTY = "E3606",
    BUNDLE_GROUP_NONE = "E3607",
    BUNDLE_DEF_NONE = "E3608",
    BUNDLE_SETTINGS_UNREADABLE = "E3609",
    BUNDLE_ON_CONFLICT = "E3610",
    BUNDLE_SETTINGS_BAD = "E3611",
    BUNDLE_WEBHOOK_DROPPED = "W3612",
    BUNDLE_WEBHOOKS_DROPPED = "W3613",
    BUNDLE_SETTINGS_COMMENTS = "W3614",
    BUNDLE_RENAME_REFS = "W3615",
    BUNDLE_CANNOT_RENAME = "W3616",
    RUN_JOB_NONE = "E3201",
    RUN_NOT_OVER = "E3202",
    RUN_NOTHING_TO_REDO = "E3203",
    RUN_PREVIOUS_JOB_NONE = "E3204",
    RUN_MARK_BAD = "E3205",
    RUN_NOTHING_AFTER = "E3206",
    RUN_SKIP_RUNNING = "E3207",
    RUN_SKIP_STRUCTURE = "E3208",
    RUN_SKIP_ENDED = "E3209",
    JOB_ENV_MISSING = "E3210",
    GROUP_MEMBER_SCHEME = "E3211",
    GROUP_MEMBER_PIN_MISSING = "E3212",
    SSH_ENV_REFUSED = "W3213",
    GROUP_WORKER_LOST = "E3214",
    GROUP_NO_EXIT = "E3215",
    FAR_SIGNAL = "E3216",
    FAR_NO_EXIT_CODE = "E3217",
    HOST_UNREADABLE = "E3218",
    SFTP_PEER_MISSING = "E3219",
    HOME_UNKNOWN = "E3221",
    DEFAULT_KEY_MISSING = "E3222",
    SECRET_MISSING = "E3223",
    SECRET_UNREADABLE = "E3224",
    RUN_AS_SECRET_MISSING = "E3225",
    MAIL_PLACE_NONE = "E3226",
    MAIL_PLACE_NAME = "E3227",
    MAIL_HOST_EMPTY = "E3228",
    MAIL_PORT_MISSING = "E3229",
    MAIL_FROM_BAD = "E3230",
    MAIL_PASSWORD_UNREADABLE = "E3231",
    MAIL_FILES_NONE = "E3232",
    MAIL_FILES_MANY = "E3233",
    MAIL_FILES_BIG = "E3234",
    MAIL_FILE_UNREADABLE = "E3235",
    MAIL_SEND_FAILED = "E3236",
    MAIL_JOB_PLACE = "E3237",
    SQL_CONNECT = "E3238",
    SQL_CONNECT_TRUST = "E3239",
    SQL_CONNECT_TLS = "E3240",
    SQL_REFUSED = "E3241",
    SQL_UNSUPPORTED = "E3242",
    SQL_OUT_WRITE = "E3243",
    SQL_DB_FILE_MISSING = "E3244",
    SQL_FILE_UNREADABLE = "E3245",
    SQL_NO_SCRIPT = "E3246",
    SQL_OUT_DIR = "E3247",
    SQL_ROWS_MISMATCH = "E3248",
    SQL_NOT_SQL = "E3249",
    SQL_NO_RESULT = "W3250",
    CLOUD_AUTH = "E3251",
    CLOUD_NOT_FOUND = "E3252",
    CLOUD_MISMATCH = "E3253",
    CLOUD_NETWORK = "E3254",
    CLOUD_TLS = "E3255",
    CLOUD_STATUS = "E3256",
    CLOUD_STATUS_BODY = "E3257",
    CLOUD_IO = "E3258",
    CLOUD_KEY_UNSAFE = "E3259",
    CLOUD_INTO_DIR = "E3260",
    CLOUD_NOT_JOB = "E3261",
    CLOUD_PEER_MISSING = "E3262",
    CLOUD_CRED_MISSING = "E3263",
    NOTIFY_EXPORT_BAD = "E3617",
    NOTIFY_WEBHOOK_SHAPE = "E3265",
    NOTIFY_SHAPE = "E3266",
    NOTIFY_PLAIN = "W3267",
    SESSION_WATCH_FAILED = "W3268",
    SESSION_SIGN_IN_HIDDEN = "W3269",
    SESSION_SIGN_OUT_HIDDEN = "W3270",
    SESSION_USER_HIDDEN = "W3271",
    RANGE_REVERSED = "E3272",
    RANGE_TOO_LONG = "E3273",
    DEF_NET_MISSING = "E3355",
    RUN_DATE_UNREADABLE = "E3275",
    RERUN_NEXT_NEEDS_JOB = "E3276",
    RUN_NOT_FOUND = "E3277",
    RUN_STILL_GOING = "E3278",
    RUN_OVER = "E3279",
    LOG_NOT_FOUND = "E3280",
    LOG_UNREADABLE = "E3281",
    JOB_NOT_RUNNING = "E3282",
    APPROVAL_NOT_WAITING = "E3283",
    LOG_CREATE_FAILED = "E3284",
    RUN_COMMAND_EMPTY = "E3285",
    EXE_START_FAILED = "E3286",
    SFTP_USER_UNKNOWN = "E3220",
    CHORE_KEY_EMPTY = "E3530",
    CHORE_ON_EMPTY = "E3531",
    CHORE_ON_BAD = "E3532",
    CHORE_GROUP_NAME_EMPTY = "E3533",
    CHORE_GROUP_NAME_BAD = "E3534",
    CHORE_MAX_RANGE = "E3535",
    CHORE_GROUP_ONLY = "E3536",
    CHORE_KEEP_DAYS = "E3537",
    CHORE_ZIP_INTO = "E3538",
    CHORE_LOCAL_ONLY = "E3539",
    CHORE_S3_KEY_WILDCARD = "E3540",
    CHORE_FETCH_FROM_NAME = "E3541",
    CHORE_S3_WILDCARD_FOLDER = "E3542",
    CHORE_HTTP_URL = "E3543",
    CHORE_HTTP_METHOD = "E3544",
    CHORE_HTTP_BODY_METHOD = "E3545",
    CHORE_TEXT_TOO_LONG = "E3546",
    CHORE_CONTENT_TYPE_NO_BODY = "E3547",
    CHORE_HTTP_EXPECT = "E3548",
    CHORE_WINRM_CERT_MISSING = "E3549",
    CHORE_PASSWORD_MISSING = "E3550",
    CHORE_PASSWORD_SECRET = "E3551",
    CHORE_MAIL_TO_EMPTY = "E3552",
    CHORE_MAIL_ADDRESS = "E3553",
    CHORE_MAIL_SUBJECT_EMPTY = "E3554",
    CHORE_MAIL_SUBJECT_NEWLINE = "E3555",
    CHORE_FILES_MAX = "E3556",
    CHORE_FILES_EMPTY = "E3557",
    CHORE_SQL_DB_EMPTY = "E3558",
    CHORE_SQL_WINDOWS_PASSWORD = "E3559",
    CHORE_SQL_TLS_OFF = "E3560",
    CHORE_SQLITE_NO = "E3561",
    CHORE_SQL_ONE_OF = "E3562",
    CHORE_SQL_TOO_LONG = "E3563",
    CHORE_HEADER_NAME = "E3564",
    CHORE_HEADER_CONTENT_TYPE = "E3565",
    CHORE_NOT_ROOT = "E3566",
    CHORE_ADDR_PATH = "E3567",
    CHORE_WINRM_USER = "E3568",
    CHORE_S3_BUCKET = "E3569",
    CHORE_S3_KEY_CONTROL = "E3570",
    CHORE_S3_NOT_SFTP_KEY = "E3571",
    CHORE_ONLY_FOR = "E3572",
    CHORE_S3_KEY_ID_MISSING = "E3573",
    CHORE_S3_KEY_ID_SECRET_NAME = "E3574",
    CHORE_S3_KEY_ID_SPACE = "E3575",
    CHORE_S3_SECRET_KEY_MISSING = "E3576",
    CHORE_S3_REGION_MISSING = "E3577",
    CHORE_S3_REGION_BAD = "E3578",
    CHORE_S3_ENDPOINT_HOST = "E3580",
    CHORE_SQLITE_PATH = "E3581",
    CHORE_DB_SCHEME = "E3582",
    CHORE_DB_NAME_MISSING = "E3583",
    CHORE_DB_NAME_BAD = "E3584",
    CHORE_SQL_INSTANCE = "E3585",
    CHORE_DB_BACKSLASH = "E3586",
    CHORE_SQL_USER_WINDOWS = "E3587",
    CHORE_DB_USER_MISSING = "E3588",
    CHORE_ROWS = "E3589",
    CHORE_COMMAND_EMPTY = "E3590",
    CHORE_CERT_BAD = "E3591",
    CHORE_PORT_BAD = "E3592",
    CHORE_ADDR_HOST_MISSING = "E3593",
    CHORE_ADDR_CHARS = "E3594",
    CHORE_ADDR_PATH_NEWLINE = "E3595",
    CHORE_HOST_KEY_MISSING = "E3596",
    CHORE_HOST_KEY_BAD = "E3597",
    CHORE_KEY_AND_PASSWORD = "E3598",
    HOST_CHECK_STOPPED = "E3599",
    HTTP_TOKEN_SECRET_UNREADABLE = "E2177",
    REMOTE_NET_CHANGED = "E2178",
    HTTP_LISTEN_FAILED = "E3767",
    HTTP_CERT_FAILED = "E3768",
    HTTP_TAILNET_PENDING = "W3769",
    HTTP_METHOD_NOT_ALLOWED = "E3773",
    HTTP_PATH_UNKNOWN = "E3774",
    HTTP_BODY_NOT_JSON = "E3775",
    HTTP_BODY_MISSING = "E3776",
    HTTP_FIELD_MISSING = "E3777",
    HTTP_REQUEST_UNREADABLE = "E3778",
    HTTP_TO_WITHOUT_FROM = "E3779",
    HTTP_FROM_AND_NEXT = "E3780",
    HTTP_WARN_NEEDS_NEXT = "E3781",
    HTTP_RUN_ID_FORM = "E3782",
    STANDBY_NOT_SET = "E3783",
    STANDBY_NOT_ACTIVE = "E3784",
    STANDBY_NOT_POLLED = "E3785",
    GIVEBACK_MANUAL = "E3786",
    GIVEBACK_NOT_YET = "E3787",
    COPY_NOT_WRITTEN = "E3788",
    COPY_REMOTE_OLD = "E3789",
    RUNS_UNREADABLE = "E3790",
    RUNS_TOO_OLD = "W3791",
    CMD_FILE_EXISTS = "E3801",
    CMD_DELETE_DECLINED = "E3802",
    CMD_APPROVE_DECLINED = "E3803",
    CMD_STAGE_DECLINED = "E3804",
    CMD_SEND_DECLINED = "E3805",
    CMD_STAGE_SAME = "E3806",
    CMD_NEXT_INVALID = "E3807",
    CMD_IMPORT_CONFLICT = "E3808",
    CMD_SEND_INVALID = "E3809",
}

/// A sentence with no ID: a completion notice, a progress line, a heading. Its words live in `[note]` of the catalogue.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Note(&'static str);

impl Note {
    /// The key in `[note]`.
    pub fn key(self) -> &'static str {
        self.0
    }
}

macro_rules! notes {
    ($($name:ident = $key:literal),* $(,)?) => {
        $(pub const $name: Note = Note($key);)*
        /// Every note there is.
        pub const ALL_NOTES: &[Note] = &[$($name),*];
    }
}

notes! {
    N_STATUS_HEAD = "status_head",
    N_STATUS_PAUSED = "status_paused",
    N_STATUS_HELD = "status_held",
    N_STATUS_HELD_JOB = "status_held_job",
    N_STATUS_QUEUED = "status_queued",
    N_EVENTS_NOT_WATCHED = "events_not_watched",
    N_HOLD_WHILE_QUEUED = "hold_while_queued",
    N_PAUSE_WHILE_QUEUED = "pause_while_queued",
    N_HELD_DONE = "held_done",
    N_RELEASED = "released",
    N_HELD_JOB_DONE = "held_job_done",
    N_RELEASED_JOB = "released_job",
    N_UNSHIFTED = "unshifted",
    N_SHIFTED = "shifted",
    N_UNSKIPPED = "unskipped",
    N_SKIPPED_NEXT = "skipped_next",
    N_PAUSE_DONE = "pause_done",
    N_RESUME_DONE = "resume_done",
    N_SAVED = "saved",
    N_PROPOSED_UNCONFIRMED = "proposed_unconfirmed",
    N_PROPOSED_SAME = "proposed_same",
    N_STAGED = "staged",
    N_UNSTAGED = "unstaged",
    N_SWITCHED = "switched",
    N_SWITCHED_UNCONFIRMED = "switched_unconfirmed",
    N_SWITCHED_CHANGED = "switched_changed",
    N_TITLE_SWITCHED = "title_switched",
    N_TITLE_SWITCH_FAILED = "title_switch_failed",
    N_CAL_WRITTEN = "cal_written",
    N_CAL_RENAMED = "cal_renamed",
    N_CAL_DELETED = "cal_deleted",
    N_HOLIDAY_IMPORTED = "holiday_imported",
    N_RESTORED_UNCONFIRMED = "restored_unconfirmed",
    N_RESTORED_SAME = "restored_same",
    N_RENAMED = "renamed",
    N_RENAMED_REFS = "renamed_refs",
    N_RENAMED_REFS_FAILED = "renamed_refs_failed",
    N_NET_APPROVED = "net_approved",
    N_OP_RUN = "op_run",
    N_OP_IMPORT = "op_import",
    N_OP_RESTORE = "op_restore",
    N_OP_RENAME = "op_rename",
    N_OP_COPY = "op_copy",
    N_OP_REPLACE = "op_replace",
    N_OP_DELETE = "op_delete",
    N_COPIED = "copied",
    N_REPLACE_NONE = "replace_none",
    N_REPLACE_PLAN = "replace_plan",
    N_REPLACED = "replaced",
    N_REPLACE_WROTE_NONE = "replace_wrote_none",
    N_REPLACE_WROTE = "replace_wrote",
    N_DELETED = "deleted",
    N_DIFF_FOLDED = "diff_folded",
    N_DIFF_MORE = "diff_more",
    N_STOP_REPLY = "stop_reply",
    N_TITLE_ENGINE_CRASHED = "title_engine_crashed",
    N_TITLE_ENGINE_STOPPED = "title_engine_stopped",
    N_AWAKE_BUSY = "awake_busy",
    N_AWAKE_WAITING = "awake_waiting",
    N_TITLE_TOOK_OVER = "title_took_over",
    N_TITLE_BOTH = "title_both",
    N_TITLE_WAITING_TAKEOVER = "title_waiting_takeover",
    N_TITLE_GAVE_BACK = "title_gave_back",
    N_SKIP_NEXT_REASON = "skip_next_reason",
    N_QUIET_PENDING = "quiet_pending",
    N_TAKE_UNREAD = "take_unread",
    N_TAKE_SUMMARY = "take_summary",
    N_TAKE_LEFT = "take_left",
    N_STANDBY_COPIED = "standby_copied",
    N_WAIT_UNTIL_OVER = "wait_until_over",
    N_HOLIDAY_STALE = "holiday_stale",
    N_STANDBY_WAITING = "standby_waiting",
    N_PAUSED = "paused",
    N_HELD_NET = "held_net",
    N_OFF_DAY_RANGE = "off_day_range",
    N_OFF_DAY_CLOSED = "off_day_closed",
    N_EVENT_START = "event_start",
    N_EVENT_FILE = "event_file",
    N_EVENT_LOG = "event_log",
    N_EVENT_EVENTLOG = "event_eventlog",
    N_EVENT_MAIL = "event_mail",
    N_EVENT_IDLE = "event_idle",
    N_EVENT_AFTER = "event_after",
    N_EVENT_MORE = "event_more",
    N_EVENT_FIRST = "event_first",
    N_EVENT_STILL_RUNNING = "event_still_running",
    N_AFTER_SAME_DAY = "after_same_day",
    N_WATCH_EXISTING = "watch_existing",
    N_SPAN_MIN = "span_min",
    N_SPAN_SEC = "span_sec",
    N_EVENTLOG_NAME = "eventlog_name",
    N_ENGINE_STOPPED = "engine_stopped",
    N_ENGINE_CRASHED = "engine_crashed",
    N_MISSED_PC_STOPPED = "missed_pc_stopped",
    N_NOT_RUN_BECAUSE = "not_run_because",
    N_NOT_RUN_INVALID = "not_run_invalid",
    N_NOT_RUN_UNCONFIRMED = "not_run_unconfirmed",
    N_NOT_RUN_PREVIOUS = "not_run_previous",
    N_QUIET_SKIP = "quiet_skip",
    N_BACKUP_NONE = "backup_none",
    N_BACKUP_EMPTY = "backup_empty",
    N_BACKUP_DONE = "backup_done",
    N_BACKUP_ERRORS = "backup_errors",
    N_RETAIN_NONE = "retain_none",
    N_RETAIN_DONE = "retain_done",
    N_RETAIN_ARCHIVED = "retain_archived",
    N_RETAIN_PRUNED = "retain_pruned",
    N_RETAIN_ERRORS = "retain_errors",
    N_LIST_SEP = "list_sep",
    N_CONFIG_AT_LINE = "config_at_line",
    N_FILE_AT_LINE = "file_at_line",
    N_PROBLEM_CODED_AT = "problem_coded_at",
    N_PROBLEM_AT = "problem_at",
    N_BLACKOUT_REASON = "blackout_reason",
    N_BLACKOUT_REASON_WHY = "blackout_reason_why",
    N_PS_WARNING = "ps_warning",
    N_PS_VERBOSE = "ps_verbose",
    N_PS_DEBUG = "ps_debug",
    N_STANDBY_NO_DEFINITION = "standby_no_definition",
    N_STANDBY_NOT_RERUN = "standby_not_rerun",
    N_STANDBY_ALL_PASSED = "standby_all_passed",
    N_LOCK_SHARED_HELD = "lock_shared_held",
    N_LOCK_EXCLUSIVE_BY_SHARED = "lock_exclusive_by_shared",
    N_LOCK_EXCLUSIVE_BY_ONE = "lock_exclusive_by_one",
    N_LOCK_EXCLUSIVE_FULL = "lock_exclusive_full",
    N_LOCK_AND_MORE = "lock_and_more",
    N_MONTHS_WITHOUT_31 = "months_without_31",
    N_MONTHS_WITHOUT_30 = "months_without_30",
    N_MONTHS_WITHOUT_29 = "months_without_29",
    N_NEW_JOBNET_FILE = "new_jobnet_file",
    N_HOLIDAY_DEFAULT = "holiday_default",
    N_HOLIDAY_SUBSTITUTE = "holiday_substitute",
    N_HOLIDAY_CITIZENS = "holiday_citizens",
    N_HOLIDAY_NEW_YEAR = "holiday_new_year",
    N_HOLIDAY_COMING_OF_AGE = "holiday_coming_of_age",
    N_HOLIDAY_FOUNDATION = "holiday_foundation",
    N_HOLIDAY_EMPEROR_BIRTHDAY = "holiday_emperor_birthday",
    N_HOLIDAY_VERNAL_EQUINOX = "holiday_vernal_equinox",
    N_HOLIDAY_SHOWA = "holiday_showa",
    N_HOLIDAY_CONSTITUTION = "holiday_constitution",
    N_HOLIDAY_GREENERY = "holiday_greenery",
    N_HOLIDAY_CHILDRENS = "holiday_childrens",
    N_HOLIDAY_MARINE = "holiday_marine",
    N_HOLIDAY_MOUNTAIN = "holiday_mountain",
    N_HOLIDAY_RESPECT_AGED = "holiday_respect_aged",
    N_HOLIDAY_AUTUMN_EQUINOX = "holiday_autumn_equinox",
    N_HOLIDAY_SPORTS = "holiday_sports",
    N_HOLIDAY_CULTURE = "holiday_culture",
    N_HOLIDAY_LABOR_THANKS = "holiday_labor_thanks",
    N_BUNDLE_KIND_SETTINGS = "bundle_kind_settings",
    N_BUNDLE_KIND_CALENDAR = "bundle_kind_calendar",
    N_BUNDLE_KIND_GROUP = "bundle_kind_group",
    N_BUNDLE_KIND_JOBNET = "bundle_kind_jobnet",
    N_BUNDLE_ADD = "bundle_add",
    N_BUNDLE_SAME = "bundle_same",
    N_BUNDLE_REPLACE = "bundle_replace",
    N_BUNDLE_RENAME = "bundle_rename",
    N_BUNDLE_SKIP = "bundle_skip",
    N_BUNDLE_PROBLEM = "bundle_problem",
    N_RUN_LATE = "run_late",
    N_RUN_LATE_AT = "run_late_at",
    N_RUN_CARRIED = "run_carried",
    N_RUN_CARRIED_WITH = "run_carried_with",
    N_RUN_SKIPPED_BY_PERSON = "run_skipped_by_person",
    N_RUN_ENGINE_STOPPED_WAITING = "run_engine_stopped_waiting",
    N_RUN_ENGINE_STOPPED = "run_engine_stopped",
    N_RUN_FROM_SKIPPED = "run_from_skipped",
    N_RUN_FROM = "run_from",
    N_RUN_RERUN_FROM = "run_rerun_from",
    N_RUN_RERUN = "run_rerun",
    N_RUN_RERUN_AFTER = "run_rerun_after",
    N_RUN_AFTER_SKIPPED = "run_after_skipped",
    N_RUN_AFTER_WARNED = "run_after_warned",
    N_RUN_TAKEN_OVER = "run_taken_over",
    N_RUN_LEFT_RUNNING = "run_left_running",
    N_RUN_LEFT_OVER = "run_left_over",
    N_RUN_WAIT_SLOT = "run_wait_slot",
    N_RUN_EXIT_WARN = "run_exit_warn",
    N_RUN_EXIT = "run_exit",
    N_RUN_KILLED = "run_killed",
    N_RUN_TIMEOUT_OVER = "run_timeout_over",
    N_RUN_TIMEOUT = "run_timeout",
    N_RUN_START_FAILED = "run_start_failed",
    N_RUN_NO_EXIT = "run_no_exit",
    N_RUN_RETRIED = "run_retried",
    N_RUN_CHILD_WAITING = "run_child_waiting",
    N_RUN_CHILD_FAILED = "run_child_failed",
    N_RUN_CHILD_WARNED = "run_child_warned",
    N_RUN_NO_EXIT_CODE = "run_no_exit_code",
    N_RUN_PRED_FAILED = "run_pred_failed",
    N_RUN_LINE_UNMET = "run_line_unmet",
    N_RUN_WAITING_FOR = "run_waiting_for",
    N_RUN_LIST_JOIN = "run_list_join",
    N_CHORE_SHARED_DIFFERS = "chore_shared_differs",
    N_HOST_SHAPE_SFTP = "host_shape_sftp",
    N_HOST_SHAPE_WINRM = "host_shape_winrm",
    N_PASS_FAILED_CODED = "pass_failed_coded",
    N_STOPPED_SSH = "ssh_stopped",
    N_STOPPED_WINRM = "winrm_stopped",
    N_GROUP_START = "group_start",
    N_GROUP_WORST = "group_worst",
    N_GROUP_BEST = "group_best",
    N_GROUP_NOT_STARTED = "group_not_started",
    N_GROUP_MORE = "group_more",
    N_NET_REFUSED = "net_refused",
    N_NET_NO_ANSWER = "net_no_answer",
    N_NET_CUT = "net_cut",
    N_NET_NO_ROUTE = "net_no_route",
    N_NET_HOST_NOT_FOUND = "net_host_not_found",
    N_NET_OS_ERROR = "net_os_error",
    N_NET_FAILED = "net_failed",
    N_NET_HOST_NOT_FOUND_WITH = "net_host_not_found_with",
    N_NET_NO_ADDRESS = "net_no_address",
    N_STOPPED_MAIL = "mail_stopped",
    N_MAIL_NO_NAMES = "mail_no_names",
    N_MAIL_NAMES = "mail_names",
    N_STOPPED_SQL = "sql_stopped",
    N_SQL_TIMEOUT = "sql_timeout",
    N_SQL_RESULT_ROWS = "sql_result_rows",
    N_SQL_CHANGED_ROWS = "sql_changed_rows",
    N_STOPPED_CLOUD = "cloud_stopped",
    N_CLOUD_TIMEOUT = "cloud_timeout",
    N_CLOUD_UNVERIFIED = "cloud_unverified",
    N_NOTIFY_LATE = "notify_late",
    N_SESSION_SIGN_IN = "session_sign_in",
    N_SESSION_SIGN_OUT = "session_sign_out",
    N_SESSION_LOCK = "session_lock",
    N_SESSION_UNLOCK = "session_unlock",
    N_SESSION_BY = "session_by",
    N_SESSION_UNSUPPORTED = "session_unsupported",
    N_SESSION_WATCH_FAILED = "session_watch_failed",
    N_UNMET_AC = "unmet_ac",
    N_UNMET_NETWORK = "unmet_network",
    N_UNMET_IDLE = "unmet_idle",
    N_WAIT_TIME_REACHED = "wait_time_reached",
    N_WAIT_FILE_ARRIVED = "wait_file_arrived",
    N_WAIT_FOR = "wait_for",
    N_WAIT_TIME_PASSED = "wait_time_passed",
    N_WAIT_DATE_UNREADABLE = "wait_date_unreadable",
    N_WAIT_APPROVAL = "wait_approval",
    N_APPROVED = "approved",
    N_REJECTED = "rejected",
    N_APPROVE_DONE = "approve_done",
    N_REJECT_DONE = "reject_done",
    N_SKIP_DONE = "skip_done",
    N_KILL_DONE = "kill_done",
    N_KILL_ASKED_JOB = "kill_asked_job",
    N_KILL_ASKED_RUN = "kill_asked_run",
    N_QUEUE_SKIPPED = "queue_skipped",
    N_HELD_JOB = "held_job",
    N_NOTIFIED = "notified",
    N_HTTP_BY = "http_by",
    N_RUN_AS_LOG = "run_as_log",
    N_NO_FILES_LOG = "no_files_log",
    N_LOOP_WIDTH_LOG = "loop_width_log",
    N_PASS_OF = "pass_of",
    N_PASS_OF_FILE = "pass_of_file",
    N_PASS_FAILED_CODE = "pass_failed_code",
    N_PASS_FAILED_REASON = "pass_failed_reason",
    N_LOG_CAPPED = "log_capped",
    N_RETRY_LOG = "retry_log",
    N_OUTPUT_ERROR_LOG = "output_error_log",
    N_OUTPUT_BAD_LINE = "output_bad_line",
    N_OK_FILE_LOG = "ok_file_log",
    N_HTTP_LISTEN_LAN = "http_listen_lan",
    N_HTTP_LISTEN_LOCAL = "http_listen_local",
    N_HTTP_LISTEN_LOCAL_V6 = "http_listen_local_v6",
    N_HTTP_TAIL_OPEN = "http_tail_open",
    N_HTTP_TAIL_PENDING = "http_tail_pending",
    N_HTTP_TAILNET_OPENED = "http_tailnet_opened",
    N_HTTP_AT_LOCAL = "http_at_local",
    N_HTTP_AT_LAN = "http_at_lan",
    N_HTTP_AT_TAILNET = "http_at_tailnet",
    N_HTTP_NO_DETAIL = "http_no_detail",
    N_HTTP_VIA_TAILSCALE = "http_via_tailscale",
    N_HTTP_NETS_JOIN = "http_nets_join",
    N_STANDBY_LINE_WATCHING = "standby_line_watching",
    N_STANDBY_LINE_WATCHING_MISSED = "standby_line_watching_missed",
    N_STANDBY_LINE_ACTIVE = "standby_line_active",
    N_STANDBY_BACK_NEXT = "standby_back_next",
    N_STANDBY_BACK_AUTO = "standby_back_auto",
    N_STANDBY_BACK_MANUAL = "standby_back_manual",
    N_STANDBY_TOOK_OVER = "standby_took_over",
    N_STANDBY_TOOK_OVER_UNREAD = "standby_took_over_unread",
    N_STANDBY_BOTH_ASKS = "standby_both_asks",
    N_STANDBY_BOTH_AUTO = "standby_both_auto",
    N_STANDBY_BOTH_MANUAL = "standby_both_manual",
    N_STANDBY_WAIT_GIVING = "standby_wait_giving",
    N_STANDBY_WAIT_AUTO = "standby_wait_auto",
    N_STANDBY_WAIT_MANUAL = "standby_wait_manual",
    N_STANDBY_GAVE_BACK_AUTO = "standby_gave_back_auto",
    N_STANDBY_GAVE_BACK_RELEASE = "standby_gave_back_release",
    N_STANDBY_REC_TOOK_OVER = "standby_rec_took_over",
    N_STANDBY_REC_GAVE_BACK_AUTO = "standby_rec_gave_back_auto",
    N_STANDBY_REC_DECIDED = "standby_rec_decided",
    N_STANDBY_REC_GAVE_BACK_CMD = "standby_rec_gave_back_cmd",
    N_STANDBY_REC_GAVE_BACK_RELEASE = "standby_rec_gave_back_release",
    N_STANDBY_RECORD_LINE = "standby_record_line",
    N_STANDBY_GIVEBACK_DECIDED = "standby_giveback_decided",
    N_STANDBY_GIVEBACK_WILL = "standby_giveback_will",
    N_STANDBY_GIVEBACK_DONE_NOBODY = "standby_giveback_done_nobody",
    N_STANDBY_GIVEBACK_DONE = "standby_giveback_done",
    N_PRIMARY_LINE_RUNNING = "primary_line_running",
    N_PRIMARY_LINE_ASKING = "primary_line_asking",
    N_PRIMARY_LINE_ASKING_MISSED = "primary_line_asking_missed",
    N_PRIMARY_LINE_HELD = "primary_line_held",
    N_PRIMARY_HELD = "primary_held",
    N_TITLE_STANDBY_PC = "title_standby_pc",
    N_PRIMARY_TOOK = "primary_took",
    N_PRIMARY_BACK = "primary_back",
    N_PRIMARY_NOBODY = "primary_nobody",
    N_PRIMARY_UNCLEAR = "primary_unclear",
    N_PRIMARY_HELD_TOLD = "primary_held_told",
    N_SERVICE_NONE = "service_none",
    N_SERVICE_LINE = "service_line",
    N_SERVICE_RUNNING = "service_running",
    N_SERVICE_STARTING = "service_starting",
    N_SERVICE_STOPPING = "service_stopping",
    N_SERVICE_STOPPED = "service_stopped",
    N_SERVICE_PASSWORD_ASK = "service_password_ask",
    N_SERVICE_DESCRIPTION = "service_description",
    N_SERVICE_INSTALLED = "service_installed",
    N_SERVICE_INSTALLED_TRAY = "service_installed_tray",
    N_SERVICE_INSTALLED_BUSY = "service_installed_busy",
    N_SERVICE_REMOVED = "service_removed",
    N_SERVICE_THEN_ENGINE = "service_then_engine",
    N_SERVICE_THEN_SCREEN = "service_then_screen",
    N_SERVICE_THEN_TEMP = "service_then_temp",
    N_SERVICE_STARTED = "service_started",
    N_SERVICE_STOPPED_DONE = "service_stopped_done",
    N_CHECK_OK = "check_ok",
    N_CHECK_OK_HOLIDAYS = "check_ok_holidays",
    N_CHECK_PLACE_JOB = "check_place_job",
    N_LIST_PAUSED = "list_paused",
    N_LIST_STAGED = "list_staged",
    N_SHOW_STAGED = "show_staged",
    N_APPROVE_ALREADY = "approve_already",
    N_APPROVE_ASK = "approve_ask",
    N_BACKFILL_NONE = "backfill_none",
    N_BACKFILL_PICKED = "backfill_picked",
    N_BACKFILL_GONE = "backfill_gone",
    N_BACKFILL_RUN = "backfill_run",
    N_BACKFILL_SKIP = "backfill_skip",
    N_DAY_NOT_PLANNED = "day_not_planned",
    N_DAY_NOTHING = "day_nothing",
    N_DAY_GONE = "day_gone",
    N_RUN_NONE_RUNNING = "run_none_running",
    N_NEW_WROTE = "new_wrote",
    N_VERSIONS_NONE = "versions_none",
    N_DELETE_ASK = "delete_ask",
    N_STAGE_HEAD = "stage_head",
    N_STAGE_ASK = "stage_ask",
    N_EXPORT_WROTE = "export_wrote",
    N_EXPORT_WROTE_SETTINGS = "export_wrote_settings",
    N_IMPORT_DONE = "import_done",
    N_IMPORT_NOTHING = "import_nothing",
    N_IMPORT_DRY = "import_dry",
    N_CLI_ENGINE_STOPPED = "cli_engine_stopped",
    N_STARTUP_TRAY = "startup_tray",
    N_STARTUP_ON = "startup_on",
    N_STARTUP_OFF = "startup_off",
    N_STARTUP_SET_TRAY = "startup_set_tray",
    N_STARTUP_SET_ENGINE = "startup_set_engine",
    N_STARTUP_SET_OFF = "startup_set_off",
    N_SECRET_SAVED = "secret_saved",
    N_SECRET_DELETED = "secret_deleted",
    N_SECRET_EXISTS = "secret_exists",
    N_SECRET_ABSENT = "secret_absent",
    N_PLAN_WEEKDAYS = "plan_weekdays",
    N_PLAN_TITLE = "plan_title",
    N_REMOTE_FP_HINT = "remote_fp_hint",
    N_REMOTE_NONE = "remote_none",
    N_REMOTE_ANSWERED = "remote_answered",
    N_REMOTE_PEEK_HINT = "remote_peek_hint",
    N_REMOTE_STATUS = "remote_status",
    N_REMOTE_PAUSED = "remote_paused",
    N_REMOTE_HELD = "remote_held",
    N_REMOTE_NO_NETS = "remote_no_nets",
    N_REMOTE_RUNNING = "remote_running",
    N_REMOTE_NO_RUNS = "remote_no_runs",
    N_REMOTE_RUN_DONE = "remote_run_done",
    N_REMOTE_RENAMED = "remote_renamed",
    N_REMOTE_RENAMED_ACK = "remote_renamed_ack",
    N_REMOTE_URL_CHANGED = "remote_url_changed",
    N_REMOTE_URL_RECONNECT = "remote_url_reconnect",
    N_REMOTE_REMOVED = "remote_removed",
    N_REMOTE_REMOVED_SECRET_LEFT = "remote_removed_secret_left",
    N_REMOTE_REMOVED_SECRET_KEPT = "remote_removed_secret_kept",
    N_SEND_SAME = "send_same",
    N_SEND_DIFF = "send_diff",
    N_SEND_NEW = "send_new",
    N_SEND_UNCONFIRMED_HERE = "send_unconfirmed_here",
    N_SEND_ASK = "send_ask",
    N_SEND_DONE = "send_done",
    N_SEND_DONE_SAME = "send_done_same",
    N_CALENDAR_UNCHANGED = "unchanged",
    N_USER_NONE = "user_none",
    N_USER_NETS_ALL = "user_nets_all",
    N_USER_TOKEN_OK = "user_token_ok",
    N_USER_TOKEN_MISSING = "user_token_missing",
    N_USER_TOKEN_UNREADABLE = "user_token_unreadable",
    N_USER_TOKEN_NONE = "user_token_none",
    N_USER_TOKEN_BAD = "user_token_bad",
    N_USER_SET_DONE = "user_set_done",
    N_CONFIG_WRITTEN = "config_written",
    N_USER_ROLE_DONE = "user_role_done",
    N_USER_ADMIN_CLEARED = "user_admin_cleared",
    N_USER_REMOVE_DONE = "user_remove_done",
    N_USER_REMOVE_DONE_ENGINE = "user_remove_done_engine",
    N_USER_SECRET_STAYS_WIN = "user_secret_stays_win",
    N_USER_SECRET_STAYS_LINUX = "user_secret_stays_linux",
    N_USER_LABEL_ROLE = "user_label_role",
    N_USER_LABEL_NETS = "user_label_nets",
    N_USER_LABEL_ROLES = "user_label_roles",
    N_USER_LABEL_TOKEN = "user_label_token",
    N_USER_TOKEN_HASH = "user_token_hash",
    N_USER_ADD_DONE = "user_add_done",
    N_USER_TOKEN_ONCE = "user_token_once",
    N_USER_REPLY_NOTE = "user_reply_note",
    N_USER_RESET_DONE = "user_reset_done",
    N_USER_RESET_ASK = "user_reset_ask",
    N_USER_RESET_NEED_YES = "user_reset_need_yes",
    N_USER_ASK_NAME = "user_ask_name",
    N_USER_ASK_NAME_BAD = "user_ask_name_bad",
    N_USER_ASK_NAME_EXISTS = "user_ask_name_exists",
    N_USER_ASK_ROLE = "user_ask_role",
    N_USER_ASK_ROLE_BAD = "user_ask_role_bad",
    N_USER_ROLE_VIEW = "user_role_view",
    N_USER_ROLE_RUN = "user_role_run",
    N_USER_ROLE_EDIT = "user_role_edit",
    N_USER_ROLE_ADMIN = "user_role_admin",
    N_USER_ASK_NETS_YN = "user_ask_nets_yn",
    N_USER_ASK_NETS = "user_ask_nets",
    N_USER_ASK_NETS_EMPTY = "user_ask_nets_empty",
    N_USER_PREVIEW_HEAD = "user_preview_head",
    N_USER_PREVIEW_TOKEN = "user_preview_token",
    N_USER_ASK_CONFIRM = "user_ask_confirm",
    N_USER_ADD_STOPPED = "user_add_stopped",
    N_USER_INPUT_ENDED = "user_input_ended",
    N_CFG_FILE = "cfg_file",
    N_CFG_NO_FILE = "cfg_no_file",
    N_CFG_CONCURRENCY = "cfg_concurrency",
    N_CFG_KEEP = "cfg_keep",
    N_CFG_LOG_MAX = "cfg_log_max",
    N_CFG_HTTP_NONE = "cfg_http_none",
    N_CFG_HTTP = "cfg_http",
    N_CFG_HTTP_LAN = "cfg_http_lan",
    N_CFG_HTTP_LOOPBACK = "cfg_http_loopback",
    N_CFG_YES = "cfg_yes",
    N_CFG_NO = "cfg_no",
    N_CFG_NONE = "cfg_none",
    N_CFG_USERS = "cfg_users",
    N_CFG_USERS_NONE = "cfg_users_none",
    N_CFG_REMOTE_NONE = "cfg_remote_none",
    N_CFG_REMOTE = "cfg_remote",
    N_CFG_REMOTE_UNWATCHED = "cfg_remote_unwatched",
    N_CFG_WATCH_OFF = "cfg_watch_off",
    N_CFG_NOTIFY = "cfg_notify",
    N_CFG_NOTIFY_PLACE = "cfg_notify_place",
    N_CFG_NOTIFY_WEBHOOK = "cfg_notify_webhook",
    N_CFG_NOTIFY_MAIL = "cfg_notify_mail",
    N_CFG_STANDBY = "cfg_standby",
    N_CFG_PRIMARY = "cfg_primary",
    N_CFG_BLACKOUT = "cfg_blackout",
    N_CFG_COUNTS = "cfg_counts",
}

/// A message filled in for a language. Text and action stay apart until a surface joins them.
pub struct Said {
    pub id: Id,
    pub text: String,
    pub action: String,
}

impl Said {
    /// `KOYOMI-E2101 <text> <action>`.
    pub fn line(&self) -> String {
        format!("{} {} {}", self.id.code(), self.text, self.action)
    }

    /// `KOYOMI-E2101 <text>`, for a surface with one fixed-height row.
    pub fn short(&self) -> String {
        format!("{} {}", self.id.code(), self.text)
    }
}

impl std::fmt::Display for Said {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.line())
    }
}

static EN: LazyLock<Table> = LazyLock::new(|| Table::new(TABLES, "en").0);
static JA: LazyLock<Table> = LazyLock::new(|| Table::new(TABLES, "ja").0);
static LANGUAGE: RwLock<&'static str> = RwLock::new("ja");

/// The table for `code`: `en`, else Japanese.
fn table(code: &str) -> &'static Table {
    if code == "en" {
        &EN
    } else {
        &JA
    }
}

/// The message in the process's language.
pub fn say(id: Id, args: &[&str]) -> Said {
    say_in(&language(), id, args)
}

/// The message in `code`'s language; no global state is read.
pub fn say_in(code: &str, id: Id, args: &[&str]) -> Said {
    let t = table(code);
    let n = id.number();
    Said { id, text: t.trf(&format!("{n}.text"), args), action: t.trf(&format!("{n}.action"), args) }
}

/// The note in the process's language.
pub fn note(n: Note, args: &[&str]) -> String {
    note_in(&language(), n, args)
}

/// The note in `code`'s language; no global state is read.
pub fn note_in(code: &str, n: Note, args: &[&str]) -> String {
    table(code).trf(&format!("note.{}", n.key()), args)
}

/// `say(id, args).line()`.
pub fn line(id: Id, args: &[&str]) -> String {
    say(id, args).line()
}

/// Whether `s` starts with an ID (`KOYOMI-` a letter E or W, four digits) and then a space or the end.
pub fn is_coded(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("KOYOMI-") else { return false };
    let b = rest.as_bytes();
    b.len() >= 5 && matches!(b[0], b'E' | b'W') && b[1..5].iter().all(u8::is_ascii_digit) && (b.len() == 5 || b[5] == b' ')
}

/// `s` without its leading ID, for a coded line that goes inside another message: one line, one ID.
pub fn uncoded(s: &str) -> &str {
    if !is_coded(s) {
        return s;
    }
    // "KOYOMI-E2101" is 12 bytes; a following space is dropped too.
    let rest = &s[12..];
    rest.strip_prefix(' ').unwrap_or(rest)
}

/// The language `say` and `line` use, process-wide: `en` or `ja`; anything else means `ja`.
pub fn set_language(code: &str) {
    let l = if code == "en" { "en" } else { "ja" };
    if let Ok(mut g) = LANGUAGE.write() {
        *g = l;
    }
}

/// The language `say` uses now.
pub fn language() -> String {
    LANGUAGE.read().map_or("ja", |g| *g).to_owned()
}

/// One catalogue entry, for the troubleshooting page and the window's card.
pub struct Entry {
    pub id: Id,
    pub area_name: String,
    pub when: String,
    pub text: String,
    pub cause: String,
    pub action: String,
}

/// `{0}` … `{9}` shown as `…`.
fn blanked(s: &str) -> String {
    let mut s = s.to_owned();
    for i in 0..10 {
        s = s.replace(&format!("{{{i}}}"), "…");
    }
    s
}

/// Every entry in `ALL`'s order; the holes of `text` and `action` are shown as `…`.
pub fn entries(code: &str) -> Vec<Entry> {
    let t = table(code);
    ALL.iter()
        .map(|&id| {
            let n = id.number();
            Entry {
                id,
                area_name: t.tr(&format!("area.a{}", id.area())).to_owned(),
                when: t.tr(&format!("{n}.when")).to_owned(),
                text: blanked(t.tr(&format!("{n}.text"))),
                cause: t.tr(&format!("{n}.cause")).to_owned(),
                action: blanked(t.tr(&format!("{n}.action"))),
            }
        })
        .collect()
}

/// The troubleshooting page, sorted by ID and grouped by area.
pub fn troubleshooting_markdown(code: &str) -> String {
    let t = table(code);
    let mut list = entries(code);
    list.sort_by(|a, b| (a.id.area(), a.id.number()).cmp(&(b.id.area(), b.id.number())));
    let field = |label: &str, value: &str| format!("- {}\n", t.trf("page.field", &[t.tr(label), value]));
    let mut out = format!("# {}\n\n{}\n{}\n", t.tr("page.title"), t.tr("page.intro"), t.tr("page.blanks"));
    let mut area = "";
    for e in &list {
        if e.id.area() != area {
            area = e.id.area();
            out.push_str(&format!("\n## {}\n", t.trf("page.area_head", &[&e.area_name, area])));
        }
        out.push_str(&format!("\n### {} {}\n\n", e.id.code(), e.when));
        let kind = if e.id.is_warning() { "page.warning" } else { "page.error" };
        out.push_str(&field("page.kind", t.tr(kind)));
        out.push_str(&field("page.message", &e.text));
        out.push_str(&field("page.cause", &e.cause));
        out.push_str(&field("page.action", &e.action));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncoded_drops_only_a_leading_id() {
        assert_eq!(uncoded("KOYOMI-E2101 x"), "x");
        assert_eq!(uncoded("plain text"), "plain text");
        assert_eq!(uncoded("KOYOMI-E2101"), "");
        assert_eq!(uncoded("a KOYOMI-E2101 b"), "a KOYOMI-E2101 b");
    }

    fn tables_parse() {
        for (code, _) in TABLES {
            assert!(Table::new(TABLES, code).1.is_empty(), "{code} does not parse");
        }
    }

    #[test]
    fn every_id_has_all_four_fields_in_both_languages_and_the_catalogue_has_no_stray_table() {
        tables_parse();
        for code in ["en", "ja"] {
            let t = table(code);
            for id in ALL {
                for k in ["when", "text", "cause", "action"] {
                    let key = format!("{}.{k}", id.number());
                    assert!(t.contains(&key) && !t.tr(&key).trim().is_empty(), "{code}: {key} is missing or empty");
                }
            }
            let known: Vec<&str> = ALL.iter().map(|i| i.number()).collect();
            let notes: Vec<String> = ALL_NOTES.iter().map(|n| format!("note.{}", n.key())).collect();
            for key in t.keys() {
                let head = key.split('.').next().unwrap_or("");
                if head == "note" {
                    assert!(notes.iter().any(|n| n == key), "{code}: {key} has no const in ALL_NOTES");
                } else {
                    assert!(matches!(head, "area" | "page") || known.contains(&head), "{code}: {key} is not in ALL");
                }
            }
        }
    }

    #[test]
    fn every_note_has_a_text_in_both_languages_with_the_same_holes_and_is_never_coded() {
        let holes = |s: &str| (0..10).filter(|i| s.contains(&format!("{{{i}}}"))).collect::<Vec<_>>();
        for (i, n) in ALL_NOTES.iter().enumerate() {
            assert!(!ALL_NOTES[..i].contains(n), "{} twice", n.key());
            let key = format!("note.{}", n.key());
            for code in ["en", "ja"] {
                assert!(table(code).contains(&key) && !table(code).tr(&key).is_empty(), "{code}: {key} is missing or empty");
            }
            assert_eq!(holes(EN.tr(&key)), holes(JA.tr(&key)), "{key}");
            assert!(!is_coded(JA.tr(&key)) && !is_coded(EN.tr(&key)), "{key}");
        }
        assert_eq!(note_in("ja", N_LOCK_AND_MORE, &["2"]), "ほか 2 件");
        assert_eq!(note_in("en", N_LOCK_AND_MORE, &["2"]), "and 2 more");
    }

    #[test]
    fn ids_are_unique_well_formed_and_have_an_area_name() {
        for (i, id) in ALL.iter().enumerate() {
            assert!(!ALL[..i].contains(id), "{} twice", id.number());
            let b = id.number().as_bytes();
            assert!(b.len() == 5 && matches!(b[0], b'E' | b'W') && b[1..].iter().all(u8::is_ascii_digit), "{}", id.number());
            for code in ["en", "ja"] {
                assert!(table(code).contains(&format!("area.a{}", id.area())), "{code}: no area name for {}", id.area());
            }
        }
    }

    #[test]
    fn the_four_digits_are_unique_across_letters() {
        for (i, id) in ALL.iter().enumerate() {
            assert!(ALL[..i].iter().all(|o| o.number()[1..] != id.number()[1..]), "{} shares its digits", id.number());
        }
    }

    #[test]
    fn is_in_wants_the_code_at_the_head() {
        assert!(CONNECT_FAILED.is_in("KOYOMI-E2101 x"));
        assert!(!CONNECT_FAILED.is_in("x KOYOMI-E2101"));
    }

    #[test]
    fn the_doc_is_sorted_by_id() {
        let md = troubleshooting_markdown("ja");
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for l in md.lines().filter_map(|l| l.strip_prefix("### KOYOMI-")) {
            let n = l.split(' ').next().unwrap_or("");
            seen.push((&n[1..3], n));
        }
        assert_eq!(seen.len(), ALL.len());
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    }

    #[test]
    fn the_translation_has_every_english_key_and_hole() {
        assert_eq!(ito_i18n::check::translations(TABLES), Vec::<String>::new());
    }

    #[test]
    fn japanese_text_and_action_read_as_sentences() {
        for e in entries("ja") {
            let n = e.id.number();
            let raw = |k: &str| JA.tr(&format!("{n}.{k}")).to_owned();
            let (text, action) = (raw("text"), raw("action"));
            assert!(["ます", "ません", "です", "ました"].iter().any(|w| text.contains(w)), "{n}: {text}");
            assert!(text.ends_with('。') || text.ends_with('）'), "{n}: {text}");
            assert!(["ください。", "ます。", "ません。", "です。"].iter().any(|w| action.ends_with(w)), "{n}: {action}");
        }
    }

    #[test]
    fn no_japanese_field_uses_a_retired_word() {
        for key in JA.keys() {
            let v = JA.tr(key);
            for w in RETIRED_WORDS {
                assert!(!v.contains(w), "{key} uses {w}");
            }
        }
    }

    #[test]
    fn the_line_and_the_short_form() {
        let s = say_in("ja", CALENDAR_EXISTS, &["週次"]);
        assert_eq!(s.text, "カレンダー 週次 はすでに存在します。");
        assert_eq!(s.line(), format!("KOYOMI-E4101 {} {}", s.text, s.action));
        assert_eq!(s.short(), format!("KOYOMI-E4101 {}", s.text));
        assert_eq!(s.to_string(), s.line());
        assert!(!s.short().contains(&s.action));
        assert_eq!(CONNECT_FAILED.code(), "KOYOMI-E2101");
        assert_eq!(CONNECT_FAILED.area(), "21");
        assert!(!CONNECT_FAILED.is_warning());
    }

    #[test]
    fn is_coded_wants_the_prefix_a_letter_four_digits_then_a_space_or_the_end() {
        for yes in ["KOYOMI-E2101 x", "KOYOMI-W4101", "KOYOMI-E2101 "] {
            assert!(is_coded(yes), "{yes}");
        }
        for no in ["xKOYOMI-E2101", "KOYOMI-E21 x", "KOYOMI-X2101 x", "KOYOMI-E2101x", "", "KOYOMI-"] {
            assert!(!is_coded(no), "{no}");
        }
    }

    #[test]
    fn the_two_languages_differ_without_the_global_one() {
        let (en, ja) = (say_in("en", CONNECT_FAILED, &["a", "b"]), say_in("ja", CONNECT_FAILED, &["a", "b"]));
        assert_eq!(en.text, "Cannot connect to a. (b)");
        assert_ne!(en.text, ja.text);
        assert_ne!(en.action, ja.action);
    }

    #[test]
    fn entries_show_the_holes_as_dots() {
        let e = entries("ja");
        assert_eq!(e.len(), ALL.len());
        assert_eq!(e[0].text, "接続先 … に接続できません。（…）");
        assert_eq!(e[0].area_name, "接続・認証");
    }

    #[test]
    fn docs_troubleshooting_is_up_to_date() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/troubleshooting.md");
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(on_disk, troubleshooting_markdown("ja"), "run: cargo run -p koyomi-core --example make-messages");
    }
}
