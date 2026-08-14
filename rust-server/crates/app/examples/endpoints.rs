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
    ("account/verifyage", Some("POST /age")),
    ("account/checkQuestIsDone", Some("GET /quests")),
    ("account/purchaseSkin", Some("POST /skins")),
    ("account/registerDiscord", Some("POST /discord")),
    ("account/unregisterDiscord", Some("DELETE /discord")),
    ("app/getServerXmls", Some("GET /content")),
    ("app/getTextures", Some("GET /textures")),
    ("app/getLanguageStrings", Some("GET /strings/:lang")),
    ("app/globalNews", Some("GET /news")),
    ("char/purchaseClassUnlock", Some("Store::purchase_class")),
    ("credits/add", Some("Store::grant_offer")),
    ("credits/getoffers", Some("GET /offers")),
    ("dailyLogin/fetchCalendar", Some("GET /daily")),
    ("inGameNews/getNews", Some("GET /news/game")),
    ("picture/get", Some("GET /picture/:id")),
    // An empty class in the original: no handler, no base, no answer. Reproducing it faithfully
    // means having nothing, which is what this is.
    (
        "security/gameData",
        Some("nothing: the original is an empty class"),
    ),
    ("security/securityProtocols", Some("GET /security")),
    ("weekQuest/getQuests", Some("GET /quests/weekly")),
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
