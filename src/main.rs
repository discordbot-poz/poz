#[cfg(feature = "dotenv")]
use dotenvy::dotenv;

use libvoicetext_api::{self, ApiOptions, AudioFormat};
use tracing_subscriber::{
    filter::LevelFilter, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
    EnvFilter,
};
use twilight_util::builder::embed::EmbedBuilder;

use tracing_subscriber::Layer;

use songbird::{
    shards::TwilightMap,
    tracks::TrackHandle,
    Songbird,
};
use std::{
    collections::HashMap, env, error::Error, num::NonZeroU64, sync::Arc,
    time::Duration, ops::Deref,
};
use tokio::sync::{watch, RwLock};
use twilight_cache_inmemory::{InMemoryCache, ResourceType};
use twilight_gateway::{
    Config, Event, Shard, ShardId,
};
use twilight_http::Client as HttpClient;
use twilight_model::{
    gateway::{
        payload::outgoing::update_presence::UpdatePresencePayload,
        presence::{ActivityType, MinimalActivity, Status},
        Intents,
    },
    http::attachment::Attachment, id::{Id, marker::GuildMarker},
};
use twilight_standby::Standby;

#[derive(Debug)]
struct StateRef {
    http: Arc<HttpClient>,
    trackdata: RwLock<HashMap<Id<GuildMarker>, TrackHandle>>,
    songbird: Songbird,
    standby: Standby,
}

fn tracing_init() {
    // tracing_subscriber::fmt::init();
    let fmt_layer = tracing_subscriber::fmt::layer().compact().with_filter(
        EnvFilter::builder()
            .with_default_directive(
                match cfg!(debug_assertions) {
                    true => LevelFilter::DEBUG,
                    false => LevelFilter::INFO,
                }
                .into(),
            )
            .from_env_lossy(),
    );

    let tokio_console_layer = console_subscriber::spawn();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(tokio_console_layer)
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "dotenv")]
    dotenv().expect(".env file not found");

    // Initialize the tracing subscriber.
    tracing_init();

    let token = env::var("DISCORD_TOKEN")?;

    // HTTP is separate from the gateway, so create a new client.
    let http = Arc::new(HttpClient::new(token.clone()));

    let user_id = http.current_user().await?.model().await?.id;

    let config = Config::builder(
        token.clone(),
        Intents::GUILD_MESSAGES | Intents::GUILD_VOICE_STATES | Intents::MESSAGE_CONTENT,
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

    let mut shard = Shard::with_config(ShardId::ONE, config);

    ctrlc::set_handler(move || {
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");

    let (_tx, _rx) = watch::channel(false);

    //let mut set = JoinSet::new();

    let senders = TwilightMap::new(HashMap::from([(shard.id().number(), shard.sender())]));

    let songbird = Songbird::twilight(Arc::new(senders), user_id);

    let state = Arc::new(StateRef {
        http,
        trackdata: Default::default(),
        songbird,
        standby: Standby::new(),
    });

    // Since we only care about new messages, make the cache only
    // cache new messages.
    let cache = Arc::new(
        InMemoryCache::builder()
            .resource_types(ResourceType::MESSAGE)
            .resource_types(ResourceType::VOICE_STATE)
            .build(),
    );
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

        tracing::debug!(?event, shard = ?shard.id(), "received event");
        // Update the cache with the event.
        cache.update(&event);

        tokio::spawn(handle_event(event, Arc::clone(&state), Arc::clone(&cache)));
    }

    Ok(())
}

async fn handle_event(
    event: Event,
    state: Arc<StateRef>,
    cache: Arc<InMemoryCache>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let event = Arc::new(event);

    tokio::spawn({
        let state = Arc::clone(&state);
        let event = Arc::clone(&event);
        async move {
            state.songbird.process(&event).await;
        }
    });

    match event.deref() {
        Event::Ready(ready) => {
            println!("Ready as: {}", ready.user.name);
        }
        Event::MessageCreate(msg) => match msg.content.as_str() {
            "!ping" => {
                state
                    .http
                    .create_message(msg.channel_id)
                    .content("Pong!")?
                    .await?;
            }
            "!join" => {
                let guild_id = msg.guild_id.ok_or("Can't join a non-guild channel.")?;
                let channel_id = NonZeroU64::new(1001440920299905096)
                    .ok_or("Joined voice channel must have nonzero ID.")?;

                let content = match state.songbird.join(guild_id, channel_id).await {
                    Ok(_handle) => format!("Joined <#{}>!", channel_id),
                    Err(e) => format!("Failed to join <#{}>! Why: {:?}", channel_id, e),
                };

                state
                    .http
                    .create_message(msg.channel_id)
                    .content(&content)?
                    .await?;

                let member = cache.guild_voice_states(guild_id);
                tracing::trace!(?member);
            }
            "!leave" => {
                state
                    .http
                    .create_message(msg.channel_id)
                    .content("Pong!")?
                    .await?;
            }
            "!test" => {
                let response = libvoicetext_api::get_audio_data(
                    env::var("VOICETEXT_API").unwrap(),
                    ApiOptions {
                        text: "テスト".to_owned(),
                        format: Some(AudioFormat::Ogg),
                        ..Default::default()
                    },
                    Duration::from_secs(1),
                ).await;

                match response {
                    Ok(audio_data) => {
                        state
                            .http
                            .create_message(msg.channel_id)
                            .content("test!")?
                            .attachments(&[Attachment::from_bytes(
                                "test.ogg".to_owned(),
                                audio_data.to_vec(),
                                1,
                            )])
                            .unwrap()
                            .await
                            .unwrap();
                    }
                    Err(err) => {
                        let error_embed = {
                            let builder = EmbedBuilder::new().color(0xff0000);

                            match err.status() {
                                Some(status) => {
                                    builder.title("APIリクエストエラー").description(format!(
                                        "{}: {}",
                                        status.as_u16(),
                                        status
                                            .canonical_reason()
                                            .unwrap_or("<unknown status code>")
                                    ))
                                }
                                None => builder.title("APIのリクエストに失敗しました。")
                            }.build()
                        };

                        state
                            .http
                            .create_message(msg.channel_id)
                            .embeds(&[error_embed])?
                            .await?;
                    }
                }
            }
            _ => {}
        }
        _ => {}
    }

    Ok(())
}
