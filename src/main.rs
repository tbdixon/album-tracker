use base64::encode;
use futures::executor::block_on;
use magick_rust::{magick_wand_genesis, MagickWand};
use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::io::prelude::*;
use std::process::Command;
use std::result::Result;
use std::sync::Once;

// Used to make sure MagickWand is initialized exactly once. Note that we
// do not bother shutting down, we simply exit when we're done.
static START: Once = Once::new();

// Use the GCP SDK to get a token
fn gcp_auth_token() -> String {
    assert!(env::var("GOOGLE_APPLICATION_CREDENTIALS").is_ok());
    String::from_utf8_lossy(
        &Command::new(env::var("AT_GCP_SDK").unwrap())
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
    print!("Processing image at {} -> ", image_path);
    io::stdout().flush()?;
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
async fn google_image(
    client: &reqwest::Client,
    encoded_image: &str,
) -> Result<String, Box<dyn Error>> {
    let payload = image_payload(encoded_image);
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

// Hit Discog API with to search for an album ID to be added to user collection
async fn discog_query(
    client: &reqwest::Client,
    album_query: &str,
) -> Result<String, Box<dyn Error>> {
    println!("{} -> ", album_query);
    let user_name = env::var("AT_DISCOGS_USER")?;
    let token = env::var("AT_DISCOGS_TOKEN")?;
    let resp = client
        .get(&format!(
            "https://api.discogs.com/database/search?q={}&token={}",
            album_query, token
        ))
        .header("Content-Type", "application/json; charset=utf-8")
        .header("User-Agent", format!("AlbumTracker/{}", user_name))
        .send()
        .await?
        .text()
        .await?;
    // the trait `serde_json::value::Index` is not implemented for `RangeTo<{integer}>`
    let master_id = &serde_json::from_str::<Value>(&resp)?["results"][0]["master_id"];
    let resp = client
        .get(&format!(
            "https://api.discogs.com/masters/{}/versions?format=Vinyl&sort=released&sort_order=desc&country=US&token={}",
            master_id, token
        ))
        .header("Content-Type", "application/json; charset=utf-8")
        .header("User-Agent", format!("AlbumTracker/{}", user_name))
        .send()
        .await?
        .text()
        .await?;
    let choice_1 = &serde_json::from_str::<Value>(&resp)?["versions"][0];
    let choice_2 = &serde_json::from_str::<Value>(&resp)?["versions"][1];
    let choice_3 = &serde_json::from_str::<Value>(&resp)?["versions"][2];
    let choice_4 = &serde_json::from_str::<Value>(&resp)?["versions"][3];
    let choice_5 = &serde_json::from_str::<Value>(&resp)?["versions"][4];
    let choice_6 = &serde_json::from_str::<Value>(&resp)?["versions"][5];
    let choice_7 = &serde_json::from_str::<Value>(&resp)?["versions"][6];
    let choice_8 = &serde_json::from_str::<Value>(&resp)?["versions"][7];
    let choice_9 = &serde_json::from_str::<Value>(&resp)?["versions"][8];
    let choice_10 = &serde_json::from_str::<Value>(&resp)?["versions"][9];
    let choices = vec![
        choice_1, choice_2, choice_3, choice_4, choice_5, choice_6, choice_7, choice_8, choice_9,
        choice_10,
    ];
    for (idx, choice) in choices.iter().enumerate() {
        println!(
            "{}: {} {} {} {}",
            idx, choice["title"], choice["country"], choice["released"], choice["format"]
        );
    }
    let mut resp = String::new();
    std::io::stdin().read_line(&mut resp)?;
    Ok(choices[resp.trim().parse::<usize>().unwrap()]["id"].to_string())
}

async fn discog_update(
    client: &reqwest::Client,
    discog_album_id: &str,
) -> Result<(), Box<dyn Error>> {
    let user_name = env::var("AT_DISCOGS_USER")?;
    let token = env::var("AT_DISCOGS_TOKEN")?;
    let resp = client
        .post(&format!(
            "https://api.discogs.com/users/{}/collection/folders/1/releases/{}?token={}",
            user_name, discog_album_id, token
        ))
        .header("Content-Type", "application/json; charset=utf-8")
        .header("User-Agent", format!("AlbumTracker/{}", user_name))
        .send()
        .await?
        .text()
        .await?;
    let resp = &serde_json::from_str::<Value>(&resp)?["basic_information"];
    print!(
        "created {} {} ({})",
        resp["artists"][0]["name"], resp["title"], resp["formats"][0]["name"]
    );
    io::stdout().flush()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    println!("Reading images in {}", args[1]);
    let image_files: Vec<std::path::PathBuf> = fs::read_dir(&args[1])?
        .map(|res| res.unwrap().path())
        .collect();
    let client = reqwest::Client::new();
    for image_path in image_files {
        print!(">>");
        io::stdout().flush()?;
        let path_str = &image_path.to_str().unwrap();
        if &path_str[path_str.len() - 9..] == "PROCESSED" {
            println!("Skipping processed image <<");
            continue;
        }
        let encoded_image = encode_image(path_str)?;
        let album_info = block_on(google_image(&client, &encoded_image))?;
        let discog_id = block_on(discog_query(&client, &album_info))?;
        block_on(discog_update(&client, &discog_id))?;
        fs::rename(&image_path, format!("{}_PROCESSED", path_str))?;
        println!("<<");
    }
    Ok(())
}
