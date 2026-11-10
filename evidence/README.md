# SexOS browser site evidence (2026-11-10)

- tls1/, tls2/: per-site result.json and serial.log.gz from the SexOS browser test runs (copied unchanged).
- xirtus-com.png: screenshot from the same runs.
- pages/SITE/: HTML and every external <script src> of the front page, fetched with curl and a desktop Chrome UA on 2026-11-10; file names keep the host + URL path (long segments shortened with a sha1 suffix). FETCH_LOG.txt per site, pages/manifest.json overall.
- pages/www.google.com/ also holds googlelogo_color_white_background_272x92dp.png (the image the SexOS browser fetched around the panic) and googleg_standard_color_128dp.png referenced in the HTML.
- Note: pages fetched by curl may differ from what SexOS received (different UA, cookies, locale), e.g. YouTube served webcomponents-sd.js to curl but webcomponents-all-noPatch.js to SexOS.

## Google panic context (tls1, last 30 lines before panic + panic)
```
[sexdisplay.surface.tab.info] surface=359 tabs=1 active=0 chrome=57
[sexdisplay.frame.light.chrome.recv] frame=0 sid=359 close_allowed=1 flags=0x39
[silk.chrome.clear] sid=359 tab_count=1 active=0 chrome_redrawn=1
[display.text.prepared] target=9 task=9 surface=156 paint=6 bytes=236 pixels=unconfirmed
[display.text.prepared] target=12 task=12 surface=154 paint=80 bytes=544 pixels=unconfirmed
[display.text.prepared] target=15 task=15 surface=300 paint=4 bytes=320 pixels=unconfirmed
[display.text.prepared] target=3 task=3 surface=200 paint=10 bytes=95 pixels=unconfirmed
[display.app.pixels] target=42 surface=359 w=410 h=330
[sexdisplay.render.full] fb_w=1280 fb_h=800
[net.receive.stats] bytes=832 calls=8 empty=7 ticks=307 flow=0 reused=0 kept=1
[web.http] method=GET status=301 bytes=220 encoding=none chunked=0 cookies=0 ticks=309 url=https://google.com/
[sexdisplay.frame.light.startup.render] sid=201 red=enabled close_allowed=1 reason=close_allowed
[net.open.flows] made=2
[clock.source.snapshot] pd=6 task=6 slot=61 seconds=1794270752 attempts=1 precision=1 profile=q35_cmos_utc ordinal=32
[silkbar.civil.sample] sequence=31 state=1 seconds=1794270752 observed_seconds=1794270752 attempts=1 source_known=1 profile=q35_cmos_utc precision=1 snapshot=1
[silkbar.civil.publish.try] sequence=31 state=1 seconds=1794270752 held_display=1
[silkbar.civil.enqueued] sequence=31 state=1 seconds=1794270752 held_display=1
[silkbar.loop.iter_advance] old=30 new=31
[silkbar.clock.cadence.sample] ss=31 yields=0 threshold=62 sent=1 ok=1
[entropy.source.sample] pd=13 task=13 slot=60 source=rdseed attempts=1 count=8 private=1
[clock.source.snapshot] pd=13 task=13 slot=61 seconds=1794270753 attempts=1 precision=1 profile=q35_cmos_utc ordinal=33
[net.tls.agreed] flow=1 version=TLSv1_3 suite=TLS13_AES_128_GCM_SHA256 group=Some(X25519)
[sched.active] tick=17000 second_distinct=13 window_peak=12 switches=7696 domains=0x4000000be7e
[sexdisplay.frame.light.startup.render] sid=153 red=enabled close_allowed=1 reason=close_allowed
[sexdisplay.frame.light.startup.render] sid=359 red=enabled close_allowed=1 reason=close_allowed
[net.receive.stats] bytes=34990 calls=32 empty=29 ticks=308 flow=1 reused=0 kept=1
[app.program.memory] target=42 task=42 granted=33 held=351 base=0x40004a3a3000 window_pages=131072 free_pages=77694 kernel_heap_used=14944424
[web.http] method=GET status=200 bytes=90237 encoding=gzip chunked=1 cookies=3 ticks=320 url=https://www.google.com/
[web.shown] w=410 h=330 scroll=0 focus=-1 items=0 ticks=2
[net.receive.stats] bytes=783 calls=5 empty=4 ticks=30 flow=1 reused=1 kept=1
[serial.record.overflow] panic diagnostic-too-long
```

## Google panic context (tls2)
```
[sexdisplay.clock.civil.stale] sequence=25 high_water=1794271003 state=Unknown seconds=0 stale=1 ticks=14202 synthesized=0
[pdx.opcode.unknown] display type_id=0x15 caller=3
[sexdisplay.surface.tab.info] surface=359 tabs=1 active=0 chrome=57
[sexdisplay.frame.light.chrome.recv] frame=0 sid=359 close_allowed=1 flags=0x39
[silk.chrome.clear] sid=359 tab_count=1 active=0 chrome_redrawn=1
[display.text.prepared] target=9 task=9 surface=156 paint=6 bytes=236 pixels=unconfirmed
[display.text.prepared] target=15 task=15 surface=300 paint=4 bytes=320 pixels=unconfirmed
[display.text.prepared] target=12 task=12 surface=154 paint=80 bytes=544 pixels=unconfirmed
[display.text.prepared] target=3 task=3 surface=200 paint=10 bytes=95 pixels=unconfirmed
[sexdisplay.clock.civil.applied] pd=6 task=6 sequence=26 state=Snapshot seconds=1794271003 high_water=1794271003 stale=0 source_known=1 source=q35_cmos_utc precision=1 snapshot_only=1 pixels=unconfirmed repaint=1
[silk.de.update.recv] kind=7 slot=0
[silk.de.update.apply.ok] kind=7 slot=0
[sexdisplay.bell.render] total=3 redacted=0 flags=0x1
[display.app.pixels] target=42 surface=359 w=410 h=330
[sexdisplay.render.full] fb_w=1280 fb_h=800
[sexdisplay.frame.light.startup.render] sid=201 red=enabled close_allowed=1 reason=close_allowed
[net.receive.stats] bytes=832 calls=6 empty=5 ticks=275 flow=0 reused=0 kept=1
[web.http] method=GET status=301 bytes=220 encoding=none chunked=0 cookies=0 ticks=275 url=https://google.com/
[net.open.flows] made=2
[scheduler.self_wait.park] task=5 ticks=1 deadline=14258 count=32768
[entropy.source.sample] pd=13 task=13 slot=60 source=rdseed attempts=1 count=8 private=1
[clock.source.snapshot] pd=13 task=13 slot=61 seconds=1794271004 attempts=1 precision=1 profile=q35_cmos_utc ordinal=28
[net.tls.agreed] flow=1 version=TLSv1_3 suite=TLS13_AES_128_GCM_SHA256 group=Some(X25519)
[sexdisplay.frame.light.startup.render] sid=153 red=enabled close_allowed=1 reason=close_allowed
[sexdisplay.frame.light.startup.render] sid=359 red=enabled close_allowed=1 reason=close_allowed
[net.receive.stats] bytes=34311 calls=22 empty=17 ticks=199 flow=1 reused=0 kept=1
[app.program.memory] target=42 task=42 granted=33 held=351 base=0x40004a3a3000 window_pages=131072 free_pages=77694 kernel_heap_used=14944424
[web.http] method=GET status=200 bytes=90231 encoding=gzip chunked=1 cookies=3 ticks=207 url=https://www.google.com/
[web.shown] w=410 h=330 scroll=0 focus=-1 items=0 ticks=2
[net.receive.stats] bytes=783 calls=4 empty=3 ticks=21 flow=1 reused=1 kept=1
[serial.record.overflow] panic diagnostic-too-long
```
