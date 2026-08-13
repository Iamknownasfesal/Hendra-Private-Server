//! Which of the original server's endpoints this one answers.
//!
//! The same shape as `gaps.rs` and `activates.rs`: a list measured from the thing being replaced,
//! so "how far along is the app server" is a number rather than an impression.

/// Every endpoint the C# app server exposes, and what answers it here.
///
/// `None` means nothing does yet. The list is written down rather than discovered because the C#
/// tree is what is being replaced, and a list that shrank when a file was deleted would measure
/// the wrong thing.
const ENDPOINTS: &[(&str, Option<&str>)] = &[
    ("app/init", Some("GET /init")),
    ("account/register", Some("POST /register")),
    ("account/verify", Some("POST /login")),
    ("char/list", Some("GET /characters")),
    ("char/delete", Some("DELETE /characters/:id")),
    ("account/changePassword", Some("POST /password")),
    ("account/setName", Some("POST /name")),
    ("char/fame", Some("GET /fame")),
    ("fame/list", Some("GET /fame")),
    ("friends/getList", Some("GET /friends")),
    ("friends/getRequests", Some("GET /friends")),
    ("privateMessage/list", Some("GET /messages")),
    ("privateMessage/send", Some("in-world /tell")),
    ("privateMessage/delete", Some("Store::delete_message")),
    ("guild/getBoard", Some("Store::guild")),
    ("guild/setBoard", Some("Store::set_guild_board")),
    ("guild/listMembers", Some("Store::guild_members")),
    ("account/purchaseCharSlot", Some("Store::buy_item")),
    ("account/rank", Some("Store::set_admin_rank")),
    ("account/sendVerifyEmail", Some("POST /email")),
    ("account/forgotPassword", Some("POST /password/forgot")),
    ("account/resetPassword", Some("POST /password/reset")),
    ("account/verifyage", None),
    ("account/checkQuestIsDone", None),
    ("account/purchaseSkin", None),
    ("account/registerDiscord", None),
    ("account/unregisterDiscord", None),
    ("app/getServerXmls", None),
    ("app/getTextures", None),
    ("app/getLanguageStrings", None),
    ("app/globalNews", Some("GET /news")),
    ("char/purchaseClassUnlock", Some("Store::purchase_class")),
    ("credits/add", None),
    ("credits/getoffers", None),
    ("dailyLogin/fetchCalendar", Some("GET /daily")),
    ("inGameNews/getNews", Some("GET /news/game")),
    ("picture/get", None),
    ("security/gameData", None),
    ("security/securityProtocols", None),
    ("weekQuest/getQuests", None),
];

fn main() {
    let answered = ENDPOINTS.iter().filter(|(_, by)| by.is_some()).count();

    println!(
        "IMPLEMENTED: {answered} of {} endpoints ({:.0}% covered)\n",
        ENDPOINTS.len(),
        100.0 * answered as f64 / ENDPOINTS.len() as f64
    );

    for (endpoint, by) in ENDPOINTS {
        match by {
            Some(route) => println!("ok   {endpoint:<32} {route}"),
            None => println!("TODO {endpoint}"),
        }
    }
}
