//! Reader view (Kaleidoscope N5): the article without menus, side bars and comments.
use sex_web::reader;

const PAGE: &str = r##"<!doctype html><title>Moths at night | Example News</title>
<header class="masthead"><nav><a href="/">Home</a> <a href="/world">World</a> <a href="/sport">Sport</a></nav></header>
<div class="layout">
 <aside class="sidebar"><h3>Most read</h3><ul><li><a href="/a">Something else entirely happened today, a long link title</a></li><li><a href="/b">Another story that people are reading a great deal</a></li></ul></aside>
 <article class="story">
  <h1>Why moths fly towards light</h1>
  <p>For centuries people have watched moths circle lamps, candles and street lights, and wondered why they do it.</p>
  <p>New work with high-speed cameras suggests the insects are not attracted to light at all, but keep their backs turned towards it, as they would to the bright sky.</p>
  <figure><img src="/pictures/moth.jpg" alt="A moth"><figcaption>A moth near a lamp.</figcaption></figure>
  <p>Near a lamp, that rule makes them tilt, turn and stall, so they end up trapped in orbits around it, the researchers say.</p>
  <script>track('story')</script>
  <div class="share">Share this on <a href="#">everything</a>, it really helps, please do, thanks</div>
  <p>The finding could help design lighting that leaves insects alone, which matters for the many birds and bats that eat them at night.</p>
 </article>
 <section class="comments"><p>First comment! This is a long comment, with commas, that says nothing much at all.</p></section>
</div>
<footer><p>Copyright Example News, all rights reserved, everywhere, forever and ever.</p></footer>"##;

#[test]
fn keeps_the_story_and_drops_the_rest() {
    let dom = sex_web::sex_html::parse::parse(PAGE);
    let a = reader::article(&dom, "https://news.example/science/moths").expect("an article");
    assert_eq!(a.title, "Why moths fly towards light");
    assert!(a.html.contains("high-speed cameras"), "{}", a.html);
    assert!(a.html.contains("leaves insects alone"), "{}", a.html);
    assert!(a.html.contains(r#"<img src="https://news.example/pictures/moth.jpg" alt="A moth">"#), "{}", a.html);
    assert!(a.html.contains("<figcaption>A moth near a lamp.</figcaption>"), "{}", a.html);
    for gone in ["Most read", "First comment", "Copyright", "Share this", "track(", "Home"] {
        assert!(!a.html.contains(gone), "{gone} in {}", a.html);
    }
    assert!(a.words > 60 && a.words < 120, "{}", a.words);
    let page = reader::page(&a, "https://news.example/science/moths");
    assert!(page.contains("<title>Why moths fly towards light</title>") && page.contains("1 min read") && page.contains(">news.example</a>"));
}

#[test]
fn a_page_without_an_article_has_no_reader_view() {
    let dom = sex_web::sex_html::parse::parse("<title>Search</title><form><input name=q></form><ul><li><a href=/1>One</a></li><li><a href=/2>Two</a></li></ul>");
    assert_eq!(reader::article(&dom, "https://x.example/"), None);
}

#[test]
fn links_and_emphasis_stay_and_text_is_escaped() {
    let body = "<div id=content><p>Read the <a href=\"/spec\">spec</a> with <em>care</em>: a &lt; b, and c &amp; d hold here, said everyone involved in the long discussion.</p>".to_string()
        + &"<p>More words in a second paragraph so that there is enough running text for an article, really.</p>".repeat(4) + "</div>";
    let dom = sex_web::sex_html::parse::parse(&body);
    let a = reader::article(&dom, "https://x.example/doc/").unwrap();
    assert!(a.html.contains(r#"Read the <a href="https://x.example/spec">spec</a> with <em>care</em>: a &lt; b, and c &amp; d"#), "{}", a.html);
}

#[test]
fn article_data_table_keeps_rows_cells_and_safe_links() {
    let body = format!(
        "<article><h1>Observation log</h1>{}<table class=\"article-data\" onclick=\"track()\"><caption>Count by place</caption>\
         <thead><tr><th>Site</th><th colspan=\"2\">Observed moths</th></tr></thead>\
         <tbody><tr><td><a href=\"/places/river\">River</a></td><td>Twenty</td><td>at dusk</td></tr>\
         <tr hidden><td>Secret row</td><td>zero</td></tr>\
         <tr><td>Forest</td><td colspan=\"999\">Thirty</td><td>at night</td></tr></tbody></table></article>",
        "<p>Careful observations of these insects across several quiet nights reveal different counts at the river and forest.</p>".repeat(4)
    );
    let dom = sex_web::sex_html::parse::parse(&body);
    let article = reader::article(&dom, "https://news.example/story").unwrap();
    assert!(article.html.contains("<tr><th>Site</th><th colspan=\"2\">Observed moths</th></tr>"), "{}", article.html);
    assert!(article.html.contains("<caption>Count by place</caption>"), "{}", article.html);
    assert!(article.html.contains("<a href=\"https://news.example/places/river\">River</a>"), "{}", article.html);
    assert!(article.html.contains("<tr><td>Forest</td><td>Thirty</td><td>at night</td></tr>"), "{}", article.html);
    assert!(!article.html.contains("Secret row") && !article.html.contains("onclick") && !article.html.contains("colspan=\"999\""));
    assert!(reader::page(&article, "https://news.example/story").contains("border-collapse:collapse"));
}

#[test]
fn article_data_table_keeps_empty_cells_in_their_columns() {
    let body = format!(
        "<article><h1>Survey</h1>{}<table><thead><tr><th>Place</th><th>Count</th></tr></thead>\
         <tbody><tr><td></td><td>Three<span hidden> hidden words do not count</span></td></tr><tr><td>Hill</td><td></td></tr></tbody></table></article>",
        "<p>The survey followed the same route each evening and recorded the weather and the count at every stop.</p>".repeat(4)
    );
    let dom = sex_web::sex_html::parse::parse(&body);
    let article = reader::article(&dom, "https://news.example/survey").unwrap();
    assert!(article.html.contains("<tr><td></td><td>Three</td></tr>"), "{}", article.html);
    assert!(article.html.contains("<tr><td>Hill</td><td></td></tr>"), "{}", article.html);
    assert!(!article.html.contains("hidden words"));
    let without_hidden = body.replace("<span hidden> hidden words do not count</span>", "");
    let plain = sex_web::sex_html::parse::parse(&without_hidden);
    assert_eq!(article.words, reader::article(&plain, "https://news.example/survey").unwrap().words);
}

#[test]
fn article_data_table_keeps_bounded_row_spans_and_nested_tables() {
    let body = format!(
        "<article><h1>Night survey</h1>{}<table><caption>Visits</caption>\
         <tr><th>Place</th><th>Visits</th></tr>\
         <tr><td rowspan='2' colspan='2'>River</td><td><table><tr><th>Time</th><th>Count</th></tr>\
         <tr><td>Dusk</td><td>Twenty</td></tr></table></td></tr>\
         <tr><td>Later</td></tr><tr><td rowspan='999' onclick='track()'>Forest</td><td>Thirty</td></tr></table></article>",
        "<p>The survey followed the same route each evening and recorded the weather and the count at every stop.</p>".repeat(4)
    );
    let dom = sex_web::sex_html::parse::parse(&body);
    let article = reader::article(&dom, "https://news.example/survey").unwrap();
    assert!(article.html.contains("<td colspan=\"2\" rowspan=\"2\">River</td>"), "{}", article.html);
    assert!(article.html.contains("<td><table><tr><th>Time</th><th>Count</th></tr><tr><td>Dusk</td><td>Twenty</td></tr></table></td>"), "{}", article.html);
    assert!(article.html.contains("<td>Forest</td>") && !article.html.contains("rowspan=\"999\""));
    assert!(!article.html.contains("onclick") && article.html.matches("Twenty").count() == 1);
    let flat = body.replace(
        "<table><tr><th>Time</th><th>Count</th></tr><tr><td>Dusk</td><td>Twenty</td></tr></table>",
        "Time Count Dusk Twenty",
    );
    let flat = sex_web::sex_html::parse::parse(&flat);
    assert_eq!(article.words, reader::article(&flat, "https://news.example/survey").unwrap().words);
}
