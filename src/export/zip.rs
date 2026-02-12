use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use vb_exchange::NamedFile;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

pub fn create_zip_from_bytes(
    files: Vec<NamedFile>,
    output_zip_path: PathBuf,
) -> std::io::Result<()> {
    // Create a file to write the ZIP archive asynchronously
    let zip_file = File::create(output_zip_path)?;

    // Initialize the ZIP writer with an async writer
    let mut zip_writer = ZipWriter::new(zip_file);

    // Set the file options for the files inside the ZIP
    let options: FileOptions<()> =
        FileOptions::default().compression_method(CompressionMethod::Deflated);

    // Add each file to the ZIP archive
    for file in files {
        // Start a new file in the ZIP archive
        zip_writer.start_file(file.name, options)?;

        // Write the file content to the ZIP archive
        zip_writer.write_all(&file.content)?;
    }

    // Finalize the ZIP file
    zip_writer.finish()?;

    Ok(())
}
