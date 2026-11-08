use std::collections::HashMap;

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

struct Pages(HashMap<String, String>);

#[async_trait::async_trait]
impl PageSource for Pages {
    async fn page(&self, url: &str) -> Option<String> {
        self.0.get(url).cloned()
    }
}

fn no_pages() -> Pages {
    Pages(HashMap::new())
}

/// Pads a fixture past the size below which a page counts as a bot check.
fn page(body: &str) -> String {
    format!(
        "<html><body>{body}{}</body></html>",
        "<!-- padding -->".repeat(2000)
    )
}

#[test]
fn links_are_classified() {
    let redgifs = |u: &str| Link::parse(u);
    for u in [
        "https://www.redgifs.com/watch/SomeClip",
        "http://v3.redgifs.com/ifr/someclip",
        "https://i.redgifs.com/i/someclip.jpg",
    ] {
        assert_eq!(redgifs(u), Link::RedGifs("someclip".into()), "{u}");
    }
    assert!(matches!(
        Link::parse("https://media.redgifs.com/SomeClip.mp4"),
        Link::File(MediaItem {
            kind: MediaType::Video,
            ..
        })
    ));
    for u in [
        "https://www.youtube.com/watch?v=abcdefghijk&t=3",
        "https://youtube.com/shorts/abcdefghijk?si=x",
        "https://youtu.be/abcdefghijk",
    ] {
        assert_eq!(Link::parse(u), Link::YouTube("abcdefghijk".into()), "{u}");
    }
    assert_eq!(
        Link::parse("https://vimeo.com/12345"),
        Link::Vimeo("12345".into())
    );
    assert_eq!(
        Link::parse("https://www.reddit.com/gallery/1abc"),
        Link::RedditPost {
            id: "1abc".into(),
            gallery: true
        }
    );
    assert_eq!(
        Link::parse("https://www.reddit.com/r/pics/comments/1abc/a_title/"),
        Link::RedditPost {
            id: "1abc".into(),
            gallery: false
        }
    );
    assert_eq!(
        Link::parse("https://www.reddit.com/media?url=https%3A%2F%2Fi.redd.it%2Fx1.jpeg"),
        Link::File(MediaItem::image("https://i.redd.it/x1.jpeg"))
    );
    assert_eq!(
        Link::parse("https://v.redd.it/vid1"),
        Link::RedditVideo("vid1".into())
    );
    assert!(matches!(
        Link::parse("https://i.imgur.com/abc.gifv"),
        Link::File(MediaItem { kind: MediaType::Video, ref src, .. }) if src == "https://i.imgur.com/abc.mp4"
    ));
    assert_eq!(Link::parse("https://blog.example/post"), Link::Other);

    assert!(is_media_url("https://www.redgifs.com/watch/someclip"));
    assert!(is_media_url("https://www.reddit.com/gallery/1abc"));
    assert!(!is_media_url(
        "https://www.reddit.com/r/pics/comments/1abc/a_title/"
    ));
    assert!(!is_media_url("https://blog.example/post"));
}

#[test]
fn feed_media_drops_previews_of_full_pictures() {
    let html = r#"<div><img src="https://i.redd.it/abc123.jpeg"></div>
        <table><tr><td><img src="https://preview.redd.it/abc123.jpeg?width=640&amp;auto=webp"></td>
        <td><img src="https://external-preview.redd.it/other.jpeg?width=640"><img src="https://t.example/p.gif" width="1" height="1"></td></tr></table>"#;
    let items = feed_media(html);
    let srcs: Vec<_> = items.iter().map(|m| m.src.as_str()).collect();
    assert_eq!(
        srcs,
        [
            "https://i.redd.it/abc123.jpeg",
            "https://external-preview.redd.it/other.jpeg?width=640"
        ]
    );
    assert_eq!(
        feed::thumbnail(&items).as_deref(),
        Some("https://external-preview.redd.it/other.jpeg?width=640")
    );
}

#[tokio::test]
async fn single_picture_posts_use_the_feed_without_fetching() {
    let resolver = Resolver::new();
    let set = resolver
        .resolve(
            "https://www.reddit.com/r/pics/comments/1abc/a_title/",
            Some(r#"<img src="https://i.redd.it/abc123.jpeg">"#),
            &no_pages(),
        )
        .await;
    assert_eq!(set.status, MediaStatus::Ready);
    assert_eq!(
        set.items,
        vec![MediaItem::image("https://i.redd.it/abc123.jpeg")]
    );
    assert_eq!(
        set.thumbnail.as_deref(),
        Some("https://i.redd.it/abc123.jpeg")
    );
}

#[tokio::test]
async fn reddit_galleries_videos_and_walls_come_from_the_post_page() {
    let gallery = page(
        r#"<shreddit-post post-type="gallery" content-href="https://www.reddit.com/gallery/1gal" nsfw>
        <gallery-carousel post-id="t3_1gal"><ul>
          <li slot="page-1"><img width="1080" height="1920" src="https://cf.preview.redd.it/t-v0-aaa.jpeg?width=640&amp;blur=40"><img class="media-lightbox-img" src="https://i.redd.it/aaa.jpeg"></li>
          <li slot="page-2"><img width="800" height="600" data-lazy-src="https://cf.preview.redd.it/t-v0-bbb.jpeg?width=640"><img class="media-lightbox-img" src="https://i.redd.it/bbb.png"></li>
        </ul></gallery-carousel></shreddit-post>
        <ac-track san="xpromo|view|bypassable_xpromo_nsfw_bypassable"></ac-track>"#,
    );
    let video = page(
        r#"<shreddit-post post-type="video" content-href="https://v.redd.it/vid1"></shreddit-post>
        <shreddit-player-2 src="https://v.redd.it/vid1/HLSPlaylist.m3u8?a=1" poster="https://preview.redd.it/p.jpeg?a=1&amp;b=2"></shreddit-player-2>"#,
    );
    let walled = page(
        r#"<shreddit-post post-type="gallery" content-href="https://www.reddit.com/gallery/1nsfw"></shreddit-post>
        <ac-track san="xpromo|click|blocking_xpromo_nsfw_blocking"></ac-track>"#,
    );
    let text = page(
        r#"<shreddit-post post-type="text" content-href="https://www.reddit.com/r/x/comments/1txt/"></shreddit-post>"#,
    );
    let challenge =
        r#"<html><form><input type="hidden" name="js_challenge" value="1"/></form></html>"#;
    let pages = Pages(HashMap::from([
        ("https://www.reddit.com/comments/1gal/".to_string(), gallery),
        ("https://www.reddit.com/comments/1vid/".to_string(), video),
        ("https://www.reddit.com/comments/1nsfw/".to_string(), walled),
        ("https://www.reddit.com/comments/1txt/".to_string(), text),
        (
            "https://www.reddit.com/comments/1chk/".to_string(),
            challenge.to_string(),
        ),
    ]));
    let resolver = Resolver::new();

    let set = resolver
        .resolve("https://www.reddit.com/gallery/1gal", None, &pages)
        .await;
    assert_eq!(set.status, MediaStatus::Ready);
    let srcs: Vec<_> = set.items.iter().map(|m| m.src.as_str()).collect();
    assert_eq!(
        srcs,
        ["https://i.redd.it/aaa.jpeg", "https://i.redd.it/bbb.png"]
    );
    assert_eq!(
        (set.items[1].width, set.items[1].height),
        (Some(800), Some(600))
    );

    let set = resolver
        .resolve(
            "https://www.reddit.com/r/x/comments/1vid/clip/",
            None,
            &pages,
        )
        .await;
    assert_eq!(set.items[0].kind, MediaType::Video);
    assert_eq!(set.items[0].src, "https://v.redd.it/vid1/HLSPlaylist.m3u8");
    assert_eq!(
        set.items[0].poster.as_deref(),
        Some("https://preview.redd.it/p.jpeg?a=1&b=2")
    );
    assert_eq!(
        set.thumbnail.as_deref(),
        Some("https://preview.redd.it/p.jpeg?a=1&b=2")
    );

    let feed = r#"<img src="https://external-preview.redd.it/thumb.jpeg?width=640">"#;
    let set = resolver
        .resolve("https://www.reddit.com/gallery/1nsfw", Some(feed), &pages)
        .await;
    assert_eq!(set.status, MediaStatus::NeedsSignIn);
    assert_eq!(
        set.thumbnail.as_deref(),
        Some("https://external-preview.redd.it/thumb.jpeg?width=640")
    );

    let set = resolver
        .resolve("https://www.reddit.com/comments/1txt/", None, &pages)
        .await;
    assert_eq!(set.status, MediaStatus::NoMedia);

    let set = resolver
        .resolve("https://www.reddit.com/comments/1chk/", None, &pages)
        .await;
    assert_eq!(set.status, MediaStatus::Failed);
}

async fn mock_redgifs() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/auth/temporary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "t1"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/gifs/clip"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"gif": {
            "type": 1, "width": 1920, "height": 1080, "duration": 12.5, "hasAudio": true, "gallery": null,
            "urls": {"hd": "https://media.example/Clip.mp4", "sd": "https://media.example/Clip-mobile.mp4", "poster": "https://media.example/Clip-poster.jpg"}
        }})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/gifs/photo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"gif": {
            "type": 2, "width": 800, "height": 1200, "duration": null, "hasAudio": false, "gallery": "set1",
            "urls": {"hd": "https://media.example/Photo.jpg"}
        }})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/gallery/set1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"gifs": [
            {"type": 2, "urls": {"hd": "https://media.example/Photo.jpg"}},
            {"type": 1, "duration": 3.0, "urls": {"hd": "https://media.example/Two.mp4", "poster": "https://media.example/Two.jpg"}}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/gifs/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn redgifs_clips_images_and_galleries() {
    let server = mock_redgifs().await;
    let resolver = Resolver::new().with_redgifs_api(server.uri());

    let set = resolver
        .resolve("https://www.redgifs.com/watch/clip", None, &no_pages())
        .await;
    assert_eq!(set.status, MediaStatus::Ready);
    let clip = &set.items[0];
    assert_eq!(clip.kind, MediaType::Video);
    assert_eq!(clip.src, "https://media.example/Clip.mp4");
    assert_eq!(clip.duration, Some(12.5));
    assert_eq!(
        set.thumbnail.as_deref(),
        Some("https://media.example/Clip-poster.jpg")
    );

    let set = resolver
        .resolve("https://i.redgifs.com/i/photo.jpg", None, &no_pages())
        .await;
    let kinds: Vec<_> = set.items.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, [MediaType::Image, MediaType::Video]);

    let feed = r#"<img src="https://external-preview.redd.it/x.jpeg?width=640">"#;
    let set = resolver
        .resolve(
            "https://www.redgifs.com/watch/gone",
            Some(feed),
            &no_pages(),
        )
        .await;
    assert_eq!(set.status, MediaStatus::Ready);
    assert_eq!(set.items[0].kind, MediaType::Image);

    // The token is fetched once and reused.
    let token_requests = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v2/auth/temporary")
        .count();
    assert_eq!(token_requests, 1);
}

#[tokio::test]
async fn unreachable_sites_fall_back_to_the_feed_and_say_so() {
    let resolver = Resolver::new().with_redgifs_api("http://127.0.0.1:9");
    let feed = r#"<img src="https://external-preview.redd.it/x.jpeg?width=640">"#;
    let set = resolver
        .resolve(
            "https://www.redgifs.com/watch/clip",
            Some(feed),
            &no_pages(),
        )
        .await;
    assert_eq!(set.status, MediaStatus::Failed);
    assert_eq!(set.items.len(), 1);

    let set = resolver
        .resolve("https://youtu.be/abcdefghijk", None, &no_pages())
        .await;
    assert_eq!(set.items[0].kind, MediaType::Embed);
    assert_eq!(
        set.thumbnail.as_deref(),
        Some("https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg")
    );

    let set = resolver
        .resolve(
            "https://blog.example/post",
            Some("<p>Just words</p>"),
            &no_pages(),
        )
        .await;
    assert_eq!(set.status, MediaStatus::NoMedia);
}
