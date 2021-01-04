use serde_json::{Result, Value};
use std::env;
use std::process::Command;

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
    .into_owned()
    .trim()
    .to_string()
}

fn image_payload(encoded_image: String) -> Value {
    let request = format!(
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
    );
    serde_json::from_str(&request).unwrap()
}

fn encode_image(image_path: String) -> String {
    unimplemented!()
}

fn main() {
    println!("{:?}", gcp_auth_token());
    println!("{:?}", image_payload(String::from("jalskdfjavckzljaqwer==")));
}
