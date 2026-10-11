//! Exercises the watch page, comments and channel calls against the saved
//! sign-in, without writing anything: `cargo run -p yt-core --example social -- VIDEO_ID`.

use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let video_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Vd7iuPV_AF0".into());
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let api = InnerTube::new(http, session);

    let watch = api.watch(&video_id).await?;
    let owner = watch.owner.clone().unwrap_or_default();
    println!(
        "watch: {:?} | {:?} | {:?} | likes {:?}",
        watch.title, watch.views, watch.published, watch.likes
    );
    println!(
        "owner: {} {:?} {:?} {:?} avatar={} verified={}",
        owner.name,
        owner.id,
        owner.handle,
        owner.subscribers,
        owner.avatar.is_some(),
        owner.verified
    );
    println!(
        "subscription: {:?}",
        watch.subscription.as_ref().map(|s| (
            s.subscribed,
            s.subscribe_params.is_some(),
            s.unsubscribe_params.is_some()
        ))
    );
    let links = watch
        .description
        .runs
        .iter()
        .filter(|r| r.link.is_some())
        .count();
    println!(
        "description: {} chars, {} links | related shelves: {}",
        watch.description.plain().len(),
        links,
        watch.related.len()
    );
    let entry = watch.comments.clone().expect("comments entry");
    println!(
        "comments: count {:?}, sorts {:?}",
        entry.count,
        entry
            .sorts
            .iter()
            .map(|s| (&s.title, s.selected))
            .collect::<Vec<_>>()
    );

    let page = api.comments(entry.token().unwrap(), &video_id).await?;
    println!(
        "comments page: {} comments, more: {}",
        page.comments.len(),
        page.next.is_some()
    );
    for c in page.comments.iter().take(3) {
        println!(
            "   {} | {} | likes {:?} | replies {} | thread {} | avatar {} | {:.40}",
            c.author.name,
            c.published,
            c.likes,
            c.reply_count,
            c.thread.is_some(),
            c.author.avatar.is_some(),
            c.text.plain()
        );
    }
    if let Some(next) = &page.next {
        println!(
            "page 2: {} comments",
            api.comments(next, &video_id).await?.comments.len()
        );
    }
    if let Some(with_replies) = page.comments.iter().find(|c| c.reply_count > 0) {
        let thread = api
            .comment_thread(with_replies.thread.as_ref().unwrap(), &video_id)
            .await?;
        let a = &thread.actions;
        println!(
            "thread: parent {:?}, {} replies, more {}, tokens like/unlike/dislike/undislike: {} {} {} {}",
            thread.comment.as_ref().map(|c| &c.id),
            thread.replies.len(),
            thread.next.is_some(),
            a.like.is_some(),
            a.unlike.is_some(),
            a.dislike.is_some(),
            a.undislike.is_some()
        );
    }

    let channel_id = owner.id.clone().unwrap();
    let channel = api.channel(&channel_id).await?;
    println!(
        "channel: {} {:?} {:?} banner={} avatar={} about={:?}",
        channel.channel.name,
        channel.channel.handle,
        channel.channel.subscribers,
        channel.banner.is_some(),
        channel.channel.avatar.is_some(),
        channel.about
    );
    println!(
        "   tabs {:?} | home shelves {} | subscription {:?}",
        channel.tabs.iter().map(|t| &t.title).collect::<Vec<_>>(),
        channel.home.shelves.len(),
        channel.subscription.as_ref().map(|s| s.subscribed)
    );
    if let Some(videos) = channel.tabs.iter().find(|t| t.title == "Videos") {
        let tab = api.browse_with(&channel_id, &videos.params).await?;
        println!(
            "   videos tab: {} shelves, {} videos, more {}",
            tab.shelves.len(),
            tab.shelves.iter().map(|s| s.videos.len()).sum::<usize>(),
            tab.continuation.is_some()
        );
    }
    if let Some(handle) = &owner.handle {
        println!("resolve {handle}: {}", api.resolve_handle(handle).await?);
    }
    let subs = api.subscriptions().await?;
    println!(
        "subscriptions: {} channels, {} with avatars",
        subs.len(),
        subs.iter().filter(|c| c.avatar.is_some()).count()
    );
    let account = api.account().await?.unwrap_or_default();
    println!(
        "account: handle {:?}, photo {}",
        account.handle,
        account.photo.is_some()
    );
    Ok(())
}
