use std::path::Path;
use std::time::Duration;
use tokio::io;

pub fn worker() {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            println!("Running cleanup.");

            if let Err(e) = cleanup_temp().await {
                eprintln!("Couldn't run temp cleanup: {}, stopping cleanup worker.", e);
                return;
            }
        }
    });
}

async fn cleanup_temp() -> io::Result<()> {
    let mut temp_folder_content = tokio::fs::read_dir(Path::new("data/temp")).await?;

    while let Some(entry) = temp_folder_content.next_entry().await? {
        let metadata = tokio::fs::metadata(entry.path()).await?;

        if metadata.modified()?.elapsed().unwrap() >= Duration::from_secs(600) {
            // Entry older than 10 minutes, delete
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(entry.path()).await?;
            } else {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
    }

    Ok(())
}
