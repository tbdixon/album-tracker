use base64::encode;
use futures::executor::block_on;
use magick_rust::{magick_wand_genesis, MagickWand};
use reqwest;
use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::process::Command;
use std::result::Result;
use std::sync::Once;

// Used to make sure MagickWand is initialized exactly once. Note that we
// do not bother shutting down, we simply exit when we're done.
static START: Once = Once::new();

// Use the GCP SDK to get a token
const GCP_SDK: &'static str = "/Applications/google-cloud-sdk/bin/gcloud";
fn gcp_auth_token() -> String {
    assert!(env::var("GOOGLE_APPLICATION_CREDENTIALS").is_ok());
    String::from_utf8_lossy(
        &Command::new(GCP_SDK)
            .arg("auth")
            .arg("application-default")
            .arg("print-access-token")
            .output()
            .expect("failed to get GCP token")
            .stdout,
    )
    .trim()
    .to_string()
}

// Creates the JSON request for Google query
fn image_payload(encoded_image: &str) -> String {
    format!(
        r#"
        {{
          "requests":[
            {{
              "image":{{
                "content": "{}"
              }},
              "features": [
                {{
                  "type": "WEB_DETECTION",
                  "maxResults":1
                }}
              ]
            }}
          ]
        }}"#,
        encoded_image
    )
}

// Read in and resize the image to shrink the size since our query takes the Base64 encoding of the
// image over the wire.
fn encode_image(image_path: &str) -> Result<String, Box<dyn Error>> {
    START.call_once(|| {
        magick_wand_genesis();
    });
    let wand = MagickWand::new();
    wand.read_image(image_path)?;
    wand.fit(1024, 1024);
    let image: &[u8] = &wand.write_image_blob("jpeg")?;
    Ok(encode(image))
}

// Hit the Google vision API to query for the Base64 encoded image and return the
// best guess label.
async fn google_image(encoded_image: &str) -> Result<String, Box<dyn Error>> {
    let payload = image_payload(encoded_image);
    let client = reqwest::Client::new();
    let resp = client
        .post("https://vision.googleapis.com/v1/images:annotate")
        .body(payload)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Authorization", format!("Bearer {}", gcp_auth_token()))
        .send()
        .await?
        .text()
        .await?;
    Ok(
        serde_json::from_str::<Value>(&resp)?["responses"][0]["webDetection"]["bestGuessLabels"][0]
            ["label"]
            .to_string(),
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    println!("Reading images in {}", args[1]);
    let image_files: Vec<std::path::PathBuf> = fs::read_dir(&args[1])?
        .map(|res| res.unwrap().path())
        .collect();
    println!("Processing {:?}", image_files);
    let image_files: Vec<String> = image_files
        .iter()
        .map(|path| encode_image(path.to_str().unwrap()).unwrap())
        .collect();
    let album_queries: Vec<String> = image_files
        .iter()
        .map(|image| block_on(google_image(image)).unwrap())
        .collect();
    println!("{:?}", album_queries);
    Ok(())
}
