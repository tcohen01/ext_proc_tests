use std::io::Read;
use std::path::PathBuf;

use serde::Deserialize;

use self::error::TryFromError;

pub mod client;
pub mod server;

#[derive(Deserialize, Debug)]
pub struct DummyDataConfig {
    pub request_headers: Vec<(String, String)>,
    #[serde(default)]
    pub request_body_filename: String,

    pub response_status: u32,
    pub response_headers: Vec<(String, String)>,
    #[serde(default)]
    pub response_body_filename: String,
}
pub struct DummyData {
    pub req_headers: Vec<(String, String)>,
    pub req_body: Vec<u8>,

    pub resp_status: u32,
    pub resp_headers: Vec<(String, String)>,
    pub resp_body: Vec<u8>,
}

mod error {
    use std::path::PathBuf;
    use quick_error::quick_error;

    quick_error!(
        #[derive(Debug)]
        pub enum TryFromError {
            OpenFile(file: &'static str, path: PathBuf, err: std::io::Error) {
                display("could not open {} file at '{}': {}", file, path.to_string_lossy(), err)
            }
            ReadFile(file: &'static str, path: PathBuf, err: std::io::Error) {
                display("could not read {} file at '{}': {}", file, path.to_string_lossy(), err)
            }
        }
    );
}

impl TryFrom<DummyDataConfig> for DummyData {
    type Error = TryFromError;

    fn try_from(value: DummyDataConfig) -> Result<Self, Self::Error> {
        fn abs_path(path: &str) -> PathBuf {
            let mut absolute_path = std::env::current_dir().unwrap_or_default();
            #[cfg(windows)]
            let path = path.replace("/", r"\");
            absolute_path.push(path);
            absolute_path
        }

        fn maybe_read_body(
            name: &'static str,
            path: &str,
            buf: &mut Vec<u8>,
        ) -> Result<(), TryFromError> {
            if path.is_empty() {
                return Ok(());
            }
            let mut body_file = std::fs::File::open(&path)
                .map_err(|e| TryFromError::OpenFile(name, abs_path(path), e))?;
            body_file
                .read_to_end(buf)
                .map_err(|e| TryFromError::ReadFile(name, abs_path(path), e))?;
            Ok(())
        }

        let mut req_body = Vec::new();
        maybe_read_body("request body", &value.request_body_filename, &mut req_body)?;

        let mut resp_body = Vec::new();
        maybe_read_body(
            "response body",
            &value.response_body_filename,
            &mut resp_body,
        )?;

        Ok(DummyData {
            req_headers: value.request_headers,
            req_body,
            resp_status: value.response_status,
            resp_headers: value.response_headers,
            resp_body,
        })
    }
}

#[cfg(test)]
mod fixture_gen {
    use build_html::{Html, HtmlContainer};
    use std::io::Write;

    #[test]
    #[ignore]
    fn generate_html() {
        let mut page = build_html::HtmlPage::new();
        let mut random_table = build_html::Table::new();

        random_table.add_header_row(vec!["Username", "Phrase"]);

        for _ in 0..50 {
            random_table.add_body_row(vec![
                memorable_wordlist::camel_case(16),
                memorable_wordlist::space_delimited(80),
            ]);
        }

        page.add_table(random_table);

        let mut file = std::fs::File::create("bench/fixtures/simple_response.html").unwrap();

        file.write_all(page.to_html_string().as_bytes())
            .expect("Writing to html fixture");
    }
}
