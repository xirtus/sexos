// SPDX-License-Identifier: GPL-3.0-only
//! Kodi repositories on the host: a repository addon, its addons.xml, version order,
//! the install plan with script.module dependencies, checksums and addon zips.
use sex_archive::{no_progress, ArchiveBuilder, OutputFormat};
use sex_kodi::addon::{content, Kind};
use sex_kodi::catalogue::PlanError;
use sex_kodi::check::{self, Hash};
use sex_kodi::{version, xml, Addon, Catalogue};
use std::cmp::Ordering;

const REPO_ADDON: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<!-- a repository addon, as users install it from a zip -->
<addon id="repository.example" name="Example Repo" version="1.0.2" provider-name="example">
  <extension point="xbmc.addon.repository" name="Example">
    <dir maxversion="18.9.0">
      <info compressed="false">https://example.org/leia/addons.xml</info>
      <checksum>https://example.org/leia/addons.xml.md5</checksum>
      <datadir zip="true">https://example.org/leia/zips/</datadir>
    </dir>
    <dir minversion="19.0.0">
      <info compressed="true">https://example.org/omega/addons.xml</info>
      <checksum verify="sha256">https://example.org/omega/addons.xml.sha256</checksum>
      <datadir zip="true">https://example.org/omega/zips</datadir>
      <hashes>sha256</hashes>
    </dir>
  </extension>
  <extension point="xbmc.addon.metadata">
    <summary lang="de_DE">Beispiel</summary>
    <summary lang="en_GB">Example &amp; friends</summary>
    <platform>all</platform>
  </extension>
</addon>"#;

const ADDONS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<addons>
<addon id="plugin.video.example" name="Example Video" version="2.1.0+matrix.1" provider-name="someone">
  <requires>
    <import addon="xbmc.python" version="3.0.0"/>
    <import addon="script.module.requests" version="2.22.0"/>
    <import addon="script.module.inputstreamhelper" version="0.5.0" optional="true"/>
  </requires>
  <extension point="xbmc.python.pluginsource" library="addon.py"><provides>video</provides></extension>
  <extension point="xbmc.addon.metadata">
    <summary>Watch examples</summary>
    <description lang="en_GB"><![CDATA[Line one <b>bold</b>]]></description>
    <assets><icon>resources/icon.png</icon><fanart>resources/fanart.jpg</fanart></assets>
    <path>plugin.video.example/plugin.video.example-2.1.0+matrix.1.zip</path>
  </extension>
</addon>
<addon id="plugin.video.example" name="Example Video" version="1.9.0" provider-name="someone">
  <requires><import addon="xbmc.python" version="2.25.0"/></requires>
  <extension point="xbmc.python.pluginsource" library="default.py"><provides>video</provides></extension>
</addon>
<addon id="plugin.video.example" name="Example Video" version="3.0.0~beta1" provider-name="someone">
  <requires><import addon="xbmc.python" version="3.0.0"/><import addon="script.module.requests" version="2.22.0"/></requires>
  <extension point="xbmc.python.pluginsource" library="addon.py"><provides>video</provides></extension>
</addon>
<addon id="script.module.requests" name="requests" version="2.31.0" provider-name="psf">
  <requires>
    <import addon="xbmc.python" version="3.0.0"/>
    <import addon="script.module.urllib3" version="1.26.0"/>
    <import addon="script.module.chardet"/>
    <import addon="script.module.certifi"/>
  </requires>
  <extension point="xbmc.python.module" library="lib"/>
</addon>
<addon id="script.module.urllib3" name="urllib3" version="1.26.18" provider-name="x">
  <requires><import addon="xbmc.python" version="3.0.0"/></requires>
  <extension point="xbmc.python.module" library="lib"/>
</addon>
<addon id="script.module.chardet" name="chardet" version="5.2.0" provider-name="x">
  <requires><import addon="script.module.requests"/></requires>
  <extension point="xbmc.python.module" library="lib"/>
</addon>
<addon id="script.module.certifi" name="certifi" version="2023.7.22" provider-name="x">
  <extension point="xbmc.python.module" library="lib"/>
</addon>
<addon id="plugin.audio.py2" name="Old radio" version="1.0.0" provider-name="x">
  <requires><import addon="xbmc.python" version="2.26.0"/></requires>
  <extension point="xbmc.python.pluginsource" library="default.py"><provides>audio</provides></extension>
</addon>
<addon id="plugin.video.gone" name="Gone" version="1.0.0" provider-name="x">
  <requires><import addon="script.module.nowhere"/></requires>
  <extension point="xbmc.python.pluginsource" library="default.py"><provides>video</provides></extension>
</addon>
<addon id="inputstream.adaptive" name="InputStream Adaptive" version="21.4.4" provider-name="x">
  <extension point="kodi.inputstream" library_linux="inputstream.adaptive.so"/>
  <extension point="xbmc.addon.metadata"><platform>linux android</platform></extension>
</addon>
<addon id="plugin.video.dead" name="Dead" version="1.0.0" provider-name="x">
  <extension point="xbmc.python.pluginsource" library="default.py"><provides>video</provides></extension>
  <extension point="xbmc.addon.metadata"><lifecyclestate type="broken" lang="en_GB">Site closed</lifecyclestate></extension>
</addon>
</addons>"#;

#[test]
fn xml_reader() {
    let root = xml::parse("<a x='1' y=\"&lt;&#65;&#x42;&amp;\"><!-- c --><b>t&apos;s</b><b/>tail<![CDATA[<raw>]]></a>").unwrap();
    assert_eq!(root.attr("y"), Some("<AB&"));
    assert_eq!(root.children_named("b").count(), 2);
    assert_eq!(root.child("b").unwrap().text(), "t's");
    assert_eq!(root.text(), "tail<raw>");
    assert_eq!(xml::parse("<a><b></a>"), Err(xml::Error::Mismatch(6)));
    assert_eq!(xml::parse("<a>"), Err(xml::Error::Truncated));
    assert_eq!(xml::parse(""), Err(xml::Error::Empty));
    let deep = "<a>".repeat(100) + &"</a>".repeat(100);
    assert_eq!(xml::parse(&deep), Err(xml::Error::TooDeep));
    assert_eq!(xml::unescape("caf&#233; &bogus; &"), "café &bogus; &");
    // Multi-byte text right after '&' must not split a character.
    assert_eq!(xml::unescape("&ééééééééé"), "&ééééééééé");
}

#[test]
fn versions_in_kodi_order() {
    let order = ["0.9", "1.0.0~alpha", "1.0.0~beta1", "1.0.0~beta2", "1.0.0", "1.0.0+matrix.1", "1.0.0+matrix.2", "1.0.1", "1.0.10", "2023.7.22", "1:0.1"];
    for w in order.windows(2) {
        assert_eq!(version::compare(w[0], w[1]), Ordering::Less, "{} < {}", w[0], w[1]);
        assert_eq!(version::compare(w[1], w[0]), Ordering::Greater);
    }
    assert_eq!(version::compare("1.02", "1.2"), Ordering::Equal);
    assert!(version::at_least("3.0.1", "3.0.0"));
}

#[test]
fn repository_addon() {
    let repo_addon = Addon::parse(REPO_ADDON).unwrap();
    assert_eq!(repo_addon.kinds, vec![Kind::Repository]);
    assert_eq!(repo_addon.summary, "Example & friends");
    let repo = repo_addon.repository.as_ref().unwrap();
    assert_eq!(repo.dirs.len(), 2);
    let ours: Vec<_> = repo.dirs_for_us().collect();
    assert_eq!(ours.len(), 1, "only the Kodi 19+ dir");
    let dir = ours[0];
    assert_eq!(dir.info, "https://example.org/omega/addons.xml.gz");
    assert_eq!(dir.checksum_hash, Hash::Sha256);
    let list = Addon::parse_list(ADDONS_XML).unwrap();
    let plugin = &list[0];
    assert_eq!(dir.zip_url(plugin).unwrap(), "https://example.org/omega/zips/plugin.video.example/plugin.video.example-2.1.0+matrix.1.zip");
    let (hash_url, h) = dir.zip_hash_url(&list[3]).unwrap();
    assert_eq!((hash_url.as_str(), h), ("https://example.org/omega/zips/script.module.requests/script.module.requests-2.31.0.zip.sha256", Hash::Sha256));
    // The old flat layout is one dir.
    let flat = Addon::parse(r#"<addon id="repository.old" version="1" name="Old"><extension point="xbmc.addon.repository"><info>u/addons.xml</info><checksum>u/addons.xml.md5</checksum><datadir zip="true">u</datadir></extension></addon>"#).unwrap();
    let d = &flat.repository.unwrap().dirs[0];
    assert!(d.suits("21.0.0") && d.checksum_hash == Hash::Md5 && d.hashes.is_none());
}

#[test]
fn addons_xml_entries() {
    let list = Addon::parse_list(ADDONS_XML).unwrap();
    let p = &list[0];
    assert_eq!(p.kinds, vec![Kind::Plugin { content: content::VIDEO, entry: "addon.py".into() }]);
    assert_eq!(p.requires.len(), 3);
    assert!(p.requires[2].optional);
    assert_eq!(p.description, "Line one <b>bold</b>");
    assert_eq!(p.icon.as_deref(), Some("resources/icon.png"));
    assert!(p.installable());
    assert!(list.iter().find(|a| a.id == "script.module.requests").unwrap().is_module());
    assert!(!list.iter().find(|a| a.id == "inputstream.adaptive").unwrap().installable(), "binary addon");
    assert_eq!(list.iter().find(|a| a.id == "plugin.video.dead").unwrap().broken.as_deref(), Some("Site closed"));
}

#[test]
fn catalogue_picks_newest_that_runs_and_plans_dependencies() {
    let mut cat = Catalogue::new();
    cat.add(0, Addon::parse_list(ADDONS_XML).unwrap());
    // 3.0.0~beta1 is newer than 2.1.0+matrix.1 and runs here; 1.9.0 is Python 2.
    assert_eq!(cat.get("plugin.video.example").unwrap().addon.version, "3.0.0~beta1");
    // A second repository with the final release wins.
    cat.add(1, Addon::parse_list(r#"<addons><addon id="plugin.video.example" name="Example Video" version="3.0.0"><requires><import addon="xbmc.python" version="3.0.0"/><import addon="script.module.requests" version="2.22.0"/></requires><extension point="xbmc.python.pluginsource" library="addon.py"><provides>video</provides></extension></addon></addons>"#).unwrap());
    let e = cat.get("plugin.video.example").unwrap();
    assert_eq!((e.addon.version.as_str(), e.source), ("3.0.0", 1));

    let plan: Vec<&str> = cat.plan("plugin.video.example", &[]).unwrap().iter().map(|e| e.addon.id.as_str()).collect();
    // Dependencies first; chardet's import of requests is a cycle and is tolerated;
    // the optional inputstreamhelper is in no repository and is skipped.
    assert_eq!(plan, ["script.module.urllib3", "script.module.chardet", "script.module.certifi", "script.module.requests", "plugin.video.example"]);
    let installed = vec![("script.module.urllib3".to_string(), "1.26.18".to_string()), ("script.module.certifi".to_string(), "2020.1.1".to_string())];
    let plan: Vec<&str> = cat.plan("plugin.video.example", &installed).unwrap().iter().map(|e| e.addon.id.as_str()).collect();
    assert_eq!(plan, ["script.module.chardet", "script.module.requests", "plugin.video.example"]);

    assert!(matches!(cat.plan("plugin.audio.py2", &[]), Err(PlanError::Unsupported { ref id, .. }) if id == "xbmc.python"));
    assert!(matches!(cat.plan("plugin.video.gone", &[]), Err(PlanError::Missing { ref id, ref needed_by }) if id == "script.module.nowhere" && needed_by == "plugin.video.gone"));
    assert!(matches!(cat.plan("inputstream.adaptive", &[]), Err(PlanError::NotInstallable { .. })));
    assert!(matches!(cat.plan("plugin.video.dead", &[]), Err(PlanError::Broken { .. })));
    assert!(matches!(cat.plan("plugin.video.nope", &[]), Err(PlanError::Missing { .. })));
    // Two imports of one module with different minimums: the plan installs one
    // version, and every import must accept it.
    let mut two = Catalogue::new();
    two.add(0, Addon::parse_list(r#"<addons>
<addon id="plugin.video.two" version="1" name="Two"><requires><import addon="script.module.a"/><import addon="script.module.b"/></requires><extension point="xbmc.python.pluginsource" library="a.py"/></addon>
<addon id="script.module.a" version="1" name="a"><requires><import addon="script.module.c" version="1.0"/></requires><extension point="xbmc.python.module"/></addon>
<addon id="script.module.b" version="1" name="b"><requires><import addon="script.module.c" version="2.0"/></requires><extension point="xbmc.python.module"/></addon>
<addon id="script.module.c" version="1.5" name="c"><extension point="xbmc.python.module"/></addon>
</addons>"#).unwrap());
    assert!(matches!(two.plan("plugin.video.two", &[]), Err(PlanError::TooOld { ref id, ref needed_by, .. }) if id == "script.module.c" && needed_by == "script.module.b"));

    let videos: Vec<&str> = cat.plugins(content::VIDEO).iter().map(|e| e.addon.name.as_str()).collect();
    assert_eq!(videos, ["Dead", "Example Video", "Gone"]);
    assert_eq!(cat.search("EXAMPLE video").len(), 1);
    let ups = cat.updates(&[("script.module.requests".into(), "2.25.1".into()), ("script.module.certifi".into(), "2023.7.22".into())]);
    assert_eq!(ups.len(), 1);
}

#[test]
fn checksums() {
    assert_eq!(check::hex(&check::md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(check::hex(&check::md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    let long = vec![b'a'; 1_000];
    assert_eq!(check::hex(&check::md5(&long)), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    assert_eq!(check::hex(&check::sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(check::hex(&check::sha1(&long)), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    assert!(check::verify(b"abc", Hash::Md5, "900150983CD24FB0D6963F7D28E17F72  addons.xml\n"));
    assert!(check::verify(b"abc", Hash::Sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    assert!(!check::verify(b"abd", Hash::Md5, "900150983cd24fb0d6963f7d28e17f72"));
    assert!(!check::verify(b"abc", Hash::Md5, "<html>not found</html>"));
}

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = ArchiveBuilder::new(OutputFormat::Zip);
    for (p, d) in files { b.add_file(p, d.len() as u64, 0, *d).unwrap(); }
    let mut out = Vec::new();
    b.write(&mut out, &mut no_progress).unwrap();
    out
}

#[test]
fn addon_zips() {
    let xml = br#"<addon id="plugin.video.example" version="3.0.0" name="E"><extension point="xbmc.python.pluginsource" library="addon.py"/></addon>"#;
    let good = zip(&[("plugin.video.example/addon.xml", xml), ("plugin.video.example/addon.py", b"print('hi')"), ("plugin.video.example/resources/lib/x.py", b"")]);
    let pkg = sex_kodi::package::unpack(good.clone(), "plugin.video.example", Some("3.0.0")).unwrap();
    let mut names: Vec<&str> = pkg.files.iter().map(|(p, _)| p.as_str()).collect();
    names.sort();
    assert_eq!(names, ["addon.py", "addon.xml", "resources/lib/x.py"]);
    assert_eq!(pkg.files.iter().find(|(p, _)| p == "addon.py").unwrap().1, b"print('hi')");
    use sex_kodi::package::Error;
    assert!(matches!(sex_kodi::package::unpack(good.clone(), "plugin.video.example", Some("2.0.0")), Err(Error::NotPromised { .. })));
    assert!(matches!(sex_kodi::package::unpack(good, "plugin.video.other", None), Err(Error::Outside(_))));
    // The writer refuses "..", so write a same-length name and patch it in.
    let mut escape = zip(&[("plugin.video.example/addon.xml", xml), ("plugin.video.example/QQ/QQ/evil.py", b"x")]);
    for i in 0..escape.len() - 5 {
        if &escape[i..i + 5] == b"QQ/QQ" { escape[i..i + 5].copy_from_slice(b"../.."); }
    }
    assert!(matches!(sex_kodi::package::unpack(escape, "plugin.video.example", None), Err(Error::Outside(_))));
    let bare = zip(&[("plugin.video.example", b"x")]);
    assert!(matches!(sex_kodi::package::unpack(bare, "plugin.video.example", None), Err(Error::Outside(_))));
    let twice = zip(&[("plugin.video.example/addon.xml", xml), ("plugin.video.example/addon.xml", b"<addon id='plugin.video.example' version='9'/>")]);
    assert!(matches!(sex_kodi::package::unpack(twice, "plugin.video.example", Some("3.0.0")), Err(Error::Duplicate(_))));
    let none = zip(&[("plugin.video.example/addon.py", b"x")]);
    assert!(matches!(sex_kodi::package::unpack(none, "plugin.video.example", None), Err(Error::NoAddonXml)));
    assert!(matches!(sex_kodi::package::unpack(b"not a zip".to_vec(), "x", None), Err(Error::Archive(_))));
}
