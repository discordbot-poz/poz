#[cfg(feature = "dotenv")]
use dotenvy::dotenv;

use libvoicetext_api::{self, ApiOptions, AudioFormat};
use twilight_util::builder::embed::EmbedBuilder;

use std::{env, error::Error, sync::Arc, time::Duration};
use twilight_cache_inmemory::{InMemoryCache, ResourceType};
use twilight_gateway::{Config, Event, Shard, ShardId};
use twilight_http::Client as HttpClient;
use twilight_model::{
    gateway::{
        payload::outgoing::update_presence::UpdatePresencePayload,
        presence::{ActivityType, MinimalActivity, Status},
        Intents,
    },
    http::attachment::Attachment,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "dotenv")]
    dotenv().expect(".env file not found");

    // Initialize the tracing subscriber.
    tracing_subscriber::fmt::init();

    let token = env::var("DISCORD_TOKEN")?;

    let config = Config::builder(
        token.clone(),
        Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
    )
    .presence(UpdatePresencePayload::new(
        vec![MinimalActivity {
            kind: ActivityType::Playing,
            name: format!("poz - v{}", env!("CARGO_PKG_VERSION")),
            url: None,
        }
        .into()],
        false,
        None,
        Status::Online,
    )?)
    .build();

    // Use intents to only receive guild message events.
    let mut shard = Shard::with_config(ShardId::ONE, config);

    // HTTP is separate from the gateway, so create a new client.
    let http = Arc::new(HttpClient::new(token));

    // Since we only care about new messages, make the cache only
    // cache new messages.
    let cache = InMemoryCache::builder()
        .resource_types(ResourceType::MESSAGE)
        .build();

    // Process each event as they come in.
    loop {
        let event = match shard.next_event().await {
            Ok(event) => event,
            Err(source) => {
                tracing::warn!(?source, "error receiving event");

                if source.is_fatal() {
                    break;
                }

                continue;
            }
        };

        // Update the cache with the event.
        cache.update(&event);

        tokio::spawn(handle_event(event, Arc::clone(&http)));
    }

    Ok(())
}

async fn handle_event(
    event: Event,
    http: Arc<HttpClient>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    match event {
        Event::Ready(ready) => {
            println!("Ready as: {}", ready.user.name);
        }
        Event::MessageCreate(msg) => {
            if msg.content == "!ping" {
                http.create_message(msg.channel_id)
                    .content("Pong!")?
                    .await?;
            }
            if msg.content == "!test" {
                match libvoicetext_api::get_audio_data(
                    env::var("VOICETEXT_API").unwrap(),
                    ApiOptions {
                        text: "テスト".to_owned(),
                        format: Some(AudioFormat::Ogg),
                        ..Default::default()
                    },
                    Duration::from_millis(1000),
                )
                .await
                {
                    Ok(audio_data) => {
                        http.create_message(msg.channel_id)
                            .content("test!")?
                            .attachments(&[Attachment::from_bytes(
                                "test.ogg".to_owned(),
                                audio_data,
                                1,
                            )])
                            .unwrap()
                            .await
                            .unwrap();
                    }
                    Err(err) => {
                        let error_message = {
                            let mut builder = EmbedBuilder::new().color(0xff0000);

                            match err.status() {
                                Some(status) => {
                                    builder =
                                        builder.title("APIリクエストエラー").description(format!(
                                            "{}: {}",
                                            status.as_u16(),
                                            status
                                                .canonical_reason()
                                                .unwrap_or("<unknown status code>")
                                        ));
                                }
                                None => {
                                    builder = builder.title("APIのリクエストに失敗しました。");
                                }
                            }

                            builder.build()
                        };

                        http.create_message(msg.channel_id)
                            .embeds(&[error_message])?
                            .await?;
                    }
                }
            }
        }
        // Other events here...
        _ => {}
    }

    Ok(())
}
