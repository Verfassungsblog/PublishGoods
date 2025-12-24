use crate::utils::api_helpers::{APIResponse, APIResult};

/// Search lobid GND API (https://lobid.org/gnd/api)
#[get("/api/gnd?<q>")]
pub async fn search_gnd(q: String) -> APIResult<serde_json::Value> {
    let url = format!(
        "https://lobid.org/gnd/search?q={}&format=json:preferredName",
        q
    );
    let resp = reqwest::get(&url).await?;
    Ok(resp.json::<serde_json::Value>().await?.into())
}
