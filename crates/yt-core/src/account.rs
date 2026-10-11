//! The signed-in account: name, handle and picture.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{best_image, collect};
use crate::feed::text;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
}

/// Reads the selected account from `account/accounts_list`.
#[yt_export::export("account")]
pub fn parse(resp: &Value) -> Option<Account> {
    let mut accounts = Vec::new();
    collect(resp, "accountItem", &mut |a| accounts.push(a));
    let account = accounts
        .iter()
        .find(|a| a.get("isSelected").and_then(Value::as_bool) == Some(true))
        .or(accounts.first())?;
    Some(Account {
        name: text(account.get("accountName")),
        handle: Some(text(account.get("channelHandle"))).filter(|h| !h.is_empty()),
        photo: best_image(account.pointer("/accountPhoto/thumbnails"), 216),
    })
}
