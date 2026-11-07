use crate::{GReaderError, READ, READING_LIST, Result, STARRED};

/// Identifies an independently paginated history stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryScope {
    pub key: String,
    pub view: String,
    pub unread_only: bool,
}

/// `unread` is the reading list with read items excluded, so it shares a
/// scope with `all` + unread-only.
pub fn history_scope(view: &str, unread_only: bool) -> HistoryScope {
    let source = if view == "unread" { "all" } else { view };
    let unread = unread_only || view == "unread";
    HistoryScope {
        key: format!("{source}\u{1f}{unread}"),
        view: source.to_string(),
        unread_only: unread,
    }
}

fn valid_view(view: &str) -> bool {
    matches!(view, "all" | "unread" | "starred")
        || view
            .strip_prefix("feed/")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// Builds the `stream/contents` path for one page of history, newest first,
/// without a date cutoff.
pub fn history_path(view: &str, unread_only: bool, continuation: Option<&str>) -> Result<String> {
    if !valid_view(view) {
        return Err(GReaderError::Config("无效的订阅筛选。".into()));
    }
    if let Some(c) = continuation
        && (c.is_empty() || c.len() >= 2000)
    {
        return Err(GReaderError::Config("无效的分页游标，请重新同步。".into()));
    }
    let scope = history_scope(view, unread_only);
    let stream = match scope.view.as_str() {
        "starred" => STARRED.to_string(),
        "all" => READING_LIST.to_string(),
        feed => feed.to_string(),
    };
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("output", "json")
        .append_pair("n", "100")
        .append_pair("r", "d");
    if scope.unread_only {
        query.append_pair("xt", READ);
    }
    if let Some(c) = continuation {
        query.append_pair("c", c);
    }
    Ok(format!("stream/contents/{stream}?{}", query.finish()))
}

/// Validates the continuation token returned for a page. A token equal to
/// the one requested means the service did not advance.
pub fn validate_continuation(
    returned: Option<&str>,
    requested: Option<&str>,
) -> Result<Option<String>> {
    match returned {
        None => Ok(None),
        Some(c) if c.is_empty() || c.len() >= 2000 => {
            Err(GReaderError::Format("返回了无效的分页游标。".into()))
        }
        Some(c) if Some(c) == requested => Err(GReaderError::Format(
            "没有推进历史分页，请稍后重试。".into(),
        )),
        Some(c) => Ok(Some(c.to_string())),
    }
}
