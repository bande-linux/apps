use std::time::{Duration, Instant};

use bande_textedit::{Format, MAX_FILE_BYTES, read, read_bounded};

type Outcome = Result<(), Box<dyn std::error::Error>>;

#[test]
fn hostile_documents_open_within_bounds_without_panicking_or_hanging() -> Outcome {
    let folder = tempfile::tempdir()?;
    let fifo = folder.path().join("pipe.txt");
    std::process::Command::new("mkfifo").arg(&fifo).status()?;
    assert!(read_bounded(&fifo, MAX_FILE_BYTES).is_err());

    let started = Instant::now();
    let rtf = format!("{{\\rtf1 {}deep{}", "{\\b ".repeat(200_000), "}".repeat(9));
    assert_eq!(read(rtf.into_bytes(), Format::Rtf, None)?.text, "");
    let html = format!("{}text", "<b><ul><li>".repeat(100_000));
    assert!(
        read(html.into_bytes(), Format::Html, None)?
            .text
            .ends_with("text")
    );
    let markdown = format!("{}{}", "*[`_".repeat(250_000), "](".repeat(100_000));
    assert!(read(markdown.into_bytes(), Format::Markdown, None).is_ok());
    assert!(started.elapsed() < Duration::from_secs(20));

    Ok(())
}
