use crate::data_storage::BibEntryV2;
use crate::settings::Settings;

/// Module to generate a Citation from a Link.

pub async fn get_translation(link: &str, settings: &Settings) -> Option<BibEntryV2>{
    let translation = send_translation_request(link, settings).await;
    match translation{
        Some(translation) => send_export_translation_request(translation, settings).await,
        None => None
    }
}

async fn send_translation_request(link: &str, settings: &Settings) -> Option<serde_json::Value>{
    let target = format!("{}/web", settings.zotero_translation_server);

    let client = match reqwest::ClientBuilder::new().timeout(std::time::Duration::from_secs(2)).build(){
        Ok(client) => client,
        Err(e) => {
            error!("Error creating client: {}", e);
            return None;
        }
    };
    let res = match client.post(&target).body(link.to_string()).header("Content-Type", "text/plain").send().await{
        Ok(res) => res,
        Err(e) => {
            error!("Error sending translation request: {}", e);
            return None;
        }
    };
    match res.json::<serde_json::Value>().await{
        Ok(text) => Some(text),
        Err(e) => {
            error!("Error reading translation response: {}", e);
            None
        }
    }
}

async fn send_export_translation_request(entry: serde_json::Value, settings: &Settings) -> Option<BibEntryV2>{
    let target = format!("{}/export?format=bibtex", settings.zotero_translation_server);

    let client = match reqwest::ClientBuilder::new().timeout(std::time::Duration::from_secs(2)).build(){
        Ok(client) => client,
        Err(e) => {
            error!("Error creating client: {}", e);
            return None;
        }
    };
    let res = match client.post(&target).json(&entry).header("Content-Type", "application/json").send().await{
        Ok(res) => res,
        Err(e) => {
            error!("Error sending translation request: {}", e);
            return None;
        }
    };
    let res = match res.text().await{
        Ok(text) => text,
        Err(e) => {
            error!("Error reading translation response: {}", e);
            return None
        }
    };

    let bibliography = hayagriva::io::from_biblatex_str(&res);
    let bibliography = match bibliography{
        Ok(bib) => bib,
        Err(e) => {
            error!("Error parsing bibliography: {:?}", e);
            return None
        }
    };
    let entry = bibliography.iter().next();
    match entry{
        Some(entry) => Some(entry.into()),
        None => None
    }
}

pub mod test{
    use super::*;

   /*
   Skip test because it fails on GitHub Actions
   #[tokio::test]
    async fn test_send_translation_request(){
        let settings = Settings::new().unwrap();
        let link = "https://verfassungsblog.de/polishing-broken-tribunal/";
        let res = send_translation_request(link, &settings).await;
        println!("{:?}", res);

        let res = send_export_translation_request(res.unwrap(), &settings).await;
        println!("{:?}", res);
        assert!(res.is_some());
    }

    */

    #[tokio::test]
    async fn test_send_invald_translation_request(){
        let settings = Settings::new().unwrap();
        let link = "https://asdfpjaflgj.de";
        let res = send_translation_request(link, &settings).await;
        println!("{:?}", res);
        assert!(res.is_none());
    }
}
