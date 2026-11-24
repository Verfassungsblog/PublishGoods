use crate::import::http_client;
use crate::settings::Settings;
use std::time::Duration;

pub async fn get_translation(link: &str, settings: &Settings) -> Option<Vec<hayagriva::Entry>> {
    let translation = send_translation_request(link, settings).await?;
    send_export_translation_request(translation, settings).await
}

async fn send_translation_request(link: &str, settings: &Settings) -> Option<serde_json::Value> {
    let target = format!("{}/web", settings.zotero_translation_server);

    let res = match http_client()
        .post(&target)
        .timeout(Duration::from_secs(settings.zotero_translation_timeout))
        .body(link.to_string())
        .header("Content-Type", "text/plain")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            error!("Error sending translation request: {}", e);
            return None;
        }
    };
    match res.json::<serde_json::Value>().await {
        Ok(text) => Some(text),
        Err(e) => {
            error!("Error reading translation response: {}", e);
            None
        }
    }
}

async fn send_export_translation_request(
    entry: serde_json::Value,
    settings: &Settings,
) -> Option<Vec<hayagriva::Entry>> {
    let target = format!(
        "{}/export?format=bibtex",
        settings.zotero_translation_server
    );

    let res = match http_client()
        .post(&target)
        .timeout(Duration::from_secs(settings.zotero_translation_timeout))
        .json(&entry)
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        Ok(res) => res,
        Err(e) => {
            error!("Error sending translation request: {}", e);
            return None;
        }
    };
    let res = match res.text().await {
        Ok(text) => text,
        Err(e) => {
            error!("Error reading translation response: {}", e);
            return None;
        }
    };

    let bibliography = hayagriva::io::from_biblatex_str(&res);
    match bibliography {
        Ok(bib) => Some(bib.into_iter().collect()),
        Err(e) => {
            error!("Error parsing bibliography: {:?}", e);
            None
        }
    }
}

#[cfg(test)]
pub mod test {
    use crate::import::link_converter::send_translation_request;
    use crate::settings::Settings;

    #[tokio::test]
    async fn test_send_invald_translation_request() {
        let settings = Settings::builder().unwrap();
        let link = "https://asdfpjaflgj.de";
        let res = send_translation_request(link, &settings).await;
        println!("{:?}", res);
        assert!(res.is_none());
    }
}
