use super::{MessagesAfterError, MessagesBeforeError, PostMessageError};
use crate::context::{Context, begin};
use domain::{ActorId, ChannelId, ChannelSeq, MessageId, OrganizationId};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub channel_seq: ChannelSeq,
    pub sender_actor_id: ActorId,
    pub body: String,
    pub created_at: OffsetDateTime,
}

// Debug output must not turn an assertion or request diagnostic into a body log.
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("id", &self.id)
            .field("channel_id", &self.channel_id)
            .field("channel_seq", &self.channel_seq)
            .field("sender_actor_id", &self.sender_actor_id)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PostMessage {
    Posted(Message),
    AlreadyPosted(Message),
}

struct StoredMessage {
    id: Uuid,
    channel_id: Uuid,
    channel_seq: i64,
    sender_actor_id: Uuid,
    body: String,
    created_at: OffsetDateTime,
}

impl TryFrom<StoredMessage> for Message {
    type Error = sqlx::Error;
    fn try_from(row: StoredMessage) -> Result<Self, Self::Error> {
        Ok(Self {
            id: MessageId::from_client(row.id)
                .map_err(|_| sqlx::Error::Protocol("invalid stored message id".into()))?,
            channel_id: ChannelId(row.channel_id),
            channel_seq: ChannelSeq::new(row.channel_seq)
                .map_err(|_| sqlx::Error::Protocol("invalid stored message sequence".into()))?,
            sender_actor_id: ActorId(row.sender_actor_id),
            body: row.body,
            created_at: row.created_at,
        })
    }
}

pub async fn post_message(
    pool: &PgPool,
    organization_id: OrganizationId,
    channel_id: ChannelId,
    sender: ActorId,
    id: MessageId,
    body: &str,
) -> Result<PostMessage, PostMessageError> {
    if !(1..=4000).contains(&body.chars().count()) {
        return Err(PostMessageError::InvalidBody);
    }
    let mut tx = begin(pool, Context::Actor(sender)).await?;
    let counter = sqlx::query!("UPDATE channels SET next_message_seq = next_message_seq + 1 WHERE id = $1 AND organization_id = $2 RETURNING next_message_seq", channel_id.0, organization_id.0)
        .fetch_optional(&mut *tx).await?.ok_or(PostMessageError::NotFound)?;
    let inserted = sqlx::query_as!(StoredMessage, "INSERT INTO messages (id, organization_id, channel_id, channel_seq, sender_actor_id, body) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING RETURNING id, channel_id, channel_seq, sender_actor_id, body, created_at", id.as_uuid(), organization_id.0, channel_id.0, counter.next_message_seq, sender.0, body)
        .fetch_optional(&mut *tx).await?;
    if let Some(row) = inserted {
        let message = Message::try_from(row)?;
        tx.commit().await?;
        return Ok(PostMessage::Posted(message));
    }
    // Release the counter and channel lock before looking up the committed retry.
    // A savepoint would keep that lock held during the lookup.
    tx.rollback().await?;
    let mut tx = begin(pool, Context::Actor(sender)).await?;
    sqlx::query!("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    let stored = sqlx::query_as!(StoredMessage, "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE id = $1 AND organization_id = $2 AND channel_id = $3 AND sender_actor_id = $4", id.as_uuid(), organization_id.0, channel_id.0, sender.0)
        .fetch_optional(&mut *tx).await?.ok_or(PostMessageError::IdConflict)?;
    let message = Message::try_from(stored)?;
    tx.commit().await?;
    Ok(PostMessage::AlreadyPosted(message))
}

async fn channel_exists(
    conn: &mut PgConnection,
    organization: OrganizationId,
    channel: ChannelId,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "SELECT id FROM channels WHERE organization_id = $1 AND id = $2",
        organization.0,
        channel.0
    )
    .fetch_optional(conn)
    .await?
    .is_some())
}

pub async fn messages_before(
    pool: &PgPool,
    actor: ActorId,
    organization_id: OrganizationId,
    channel_id: ChannelId,
    before: Option<ChannelSeq>,
    limit: u16,
) -> Result<Vec<Message>, MessagesBeforeError> {
    let mut tx = begin(pool, Context::Actor(actor)).await?;
    if !channel_exists(&mut tx, organization_id, channel_id).await? {
        return Err(MessagesBeforeError::NotFound);
    }
    let limit = i64::from(limit.clamp(1, 200));
    let rows = if let Some(before) = before {
        sqlx::query_as!(StoredMessage, "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 AND channel_seq < $3 ORDER BY channel_seq DESC LIMIT $4", organization_id.0, channel_id.0, before.get(), limit).fetch_all(&mut *tx).await?
    } else {
        sqlx::query_as!(StoredMessage, "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 ORDER BY channel_seq DESC LIMIT $3", organization_id.0, channel_id.0, limit).fetch_all(&mut *tx).await?
    };
    let messages = rows
        .into_iter()
        .map(Message::try_from)
        .collect::<Result<_, _>>()?;
    tx.commit().await?;
    Ok(messages)
}

pub async fn messages_after(
    pool: &PgPool,
    actor: ActorId,
    organization_id: OrganizationId,
    channel_id: ChannelId,
    after: ChannelSeq,
    limit: u16,
) -> Result<Vec<Message>, MessagesAfterError> {
    let mut tx = begin(pool, Context::Actor(actor)).await?;
    if !channel_exists(&mut tx, organization_id, channel_id).await? {
        return Err(MessagesAfterError::NotFound);
    }
    let limit = i64::from(limit.clamp(1, 200));
    let rows = sqlx::query_as!(StoredMessage, "SELECT id, channel_id, channel_seq, sender_actor_id, body, created_at FROM messages WHERE organization_id = $1 AND channel_id = $2 AND channel_seq > $3 ORDER BY channel_seq ASC LIMIT $4", organization_id.0, channel_id.0, after.get(), limit).fetch_all(&mut *tx).await?;
    let messages = rows
        .into_iter()
        .map(Message::try_from)
        .collect::<Result<_, _>>()?;
    tx.commit().await?;
    Ok(messages)
}
