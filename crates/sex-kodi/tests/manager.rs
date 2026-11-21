// SPDX-License-Identifier: GPL-3.0-only
//! The addon folder on the host: a repository zip added by address, its addons.xml
//! checked and read, an addon installed with its dependency, all through mock Net and
//! Store; then the folder read back, and tampered downloads refused.
use sex_archive::{no_progress, ArchiveBuilder, OutputFormat};
use sex_kodi::check::{self, Hash};
use sex_kodi::manager::Error;
use sex_kodi::{Manager, Net, Store};
use std::collections::BTreeMap;

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = ArchiveBuilder::new(OutputFormat::Zip);
    for (p, d) in files { b.add_file(p, d.len() as u64, 0, *d).unwrap(); }
    let mut out = Vec::new();
    b.write(&mut out, &mut no_progress).unwrap();
    out
}

const REPO: &str = r#"<addon id="repository.test" name="Test Repo" version="1.0.0" provider-name="t">
  <extension point="xbmc.addon.repository" name="Test">
    <dir minversion="19.0.0">
      <info compressed="false">http://r/omega/addons.xml</info>
      <checksum>http://r/omega/addons.xml.md5</checksum>
      <datadir zip="true">http://r/omega/zips/</datadir>
      <hashes>sha256</hashes>
    </dir>
  </extension>
</addon>"#;
const PLUGIN: &str = r#"<addon id="plugin.video.test" name="Test Videos" version="2.0.0" provider-name="t">
  <requires><import addon="xbmc.python" version="3.0.0"/><import addon="script.module.dep" version="1.0.0"/></requires>
  <extension point="xbmc.python.pluginsource" library="main.py"><provides>video</provides></extension>
  <extension point="xbmc.addon.metadata"><summary>Videos for the test</summary></extension>
</addon>"#;
const DEP: &str = r#"<addon id="script.module.dep" name="Dep" version="1.1.0" provider-name="t">
  <requires><import addon="xbmc.python" version="3.0.0"/></requires>
  <extension point="xbmc.python.module" library="lib"/>
</addon>"#;

#[derive(Default)]
struct Web(BTreeMap<String, Vec<u8>>, Vec<String>);
impl Net for Web {
    fn get(&mut self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        self.1.push(url.to_string());
        match self.0.get(url) { Some(b) if b.len() <= max => Ok(b.clone()), Some(_) => Err("too big".into()), None => Err("404".into()) }
    }
}
#[derive(Default)]
struct Folder(BTreeMap<String, Vec<u8>>);
impl Store for Folder {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> { self.0.get(path).cloned() }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> { self.0.insert(path.to_string(), data.to_vec()); Ok(()) }
}

fn web() -> Web {
    let mut w = Web::default();
    let index = format!("<addons>\n{REPO}\n{PLUGIN}\n{DEP}\n</addons>");
    w.0.insert("http://r/repository.test-1.0.0.zip".into(), zip(&[("repository.test/addon.xml", REPO.as_bytes()), ("repository.test/icon.png", b"png")]));
    w.0.insert("http://r/omega/addons.xml.md5".into(), check::hex(&check::md5(index.as_bytes())).into_bytes());
    w.0.insert("http://r/omega/addons.xml".into(), index.into_bytes());
    for (id, v, xml, extra) in [("plugin.video.test", "2.0.0", PLUGIN, ("main.py", &b"print('hi')"[..])), ("script.module.dep", "1.1.0", DEP, ("lib/dep.py", &b"X = 1"[..]))] {
        let z = zip(&[(&format!("{id}/addon.xml"), xml.as_bytes()), (&format!("{id}/{}", extra.0), extra.1)]);
        let url = format!("http://r/omega/zips/{id}/{id}-{v}.zip");
        w.0.insert(format!("{url}.sha256"), format!("{}  {id}-{v}.zip\n", check::hex(&Hash::Sha256.digest(&z))).into_bytes());
        w.0.insert(url, z);
    }
    w
}

#[test]
fn add_refresh_install_and_reload() {
    let (mut net, mut store) = (web(), Folder::default());
    let mut m = Manager::load(&mut store);
    assert!(m.installed.is_empty() && m.sources.is_empty());
    let repo = m.add_repository(&mut net, &mut store, "http://r/repository.test-1.0.0.zip").unwrap();
    assert_eq!(repo.id, "repository.test");
    assert_eq!(m.sources.len(), 1);
    assert_eq!(store.0.get("addons/repository.test/icon.png").unwrap(), b"png");
    let report = m.refresh(&mut net);
    assert_eq!(report[0].addons, Ok(3));
    let plugins = m.catalogue.plugins(sex_kodi::addon::content::VIDEO);
    assert_eq!(plugins.iter().map(|e| e.addon.id.as_str()).collect::<Vec<_>>(), ["plugin.video.test"]);
    let done = m.install(&mut net, &mut store, "plugin.video.test").unwrap();
    assert_eq!(done, [("script.module.dep".to_string(), "1.1.0".to_string()), ("plugin.video.test".to_string(), "2.0.0".to_string())]);
    assert_eq!(store.0.get("addons/plugin.video.test/main.py").unwrap(), b"print('hi')");
    assert_eq!(store.0.get("addons/script.module.dep/lib/dep.py").unwrap(), b"X = 1");
    assert_eq!(String::from_utf8(store.0["installed.txt"].clone()).unwrap(),
        "repository.test\t1.0.0\nscript.module.dep\t1.1.0\nplugin.video.test\t2.0.0\n");
    // Nothing left to do the second time.
    assert!(m.install(&mut net, &mut store, "plugin.video.test").unwrap().is_empty());
    // A later run finds the same folder.
    let mut again = Manager::load(&mut store);
    assert_eq!(again.installed.len(), 3);
    assert_eq!(again.sources, m.sources);
    assert_eq!(again.version_of("plugin.video.test"), Some("2.0.0"));
    assert_eq!(again.refresh(&mut net)[0].addons, Ok(3));
}

#[test]
fn tampered_downloads_are_refused() {
    let (mut net, mut store) = (web(), Folder::default());
    let mut m = Manager::default();
    m.add_repository(&mut net, &mut store, "http://r/repository.test-1.0.0.zip").unwrap();
    // addons.xml changed after its checksum was published.
    let mut bad = net.0["http://r/omega/addons.xml"].clone();
    bad.extend_from_slice(b"\n");
    let good = std::mem::replace(net.0.get_mut("http://r/omega/addons.xml").unwrap(), bad);
    assert!(matches!(m.refresh(&mut net)[0].addons, Err(Error::Checksum(_))));
    net.0.insert("http://r/omega/addons.xml".into(), good);
    assert_eq!(m.refresh(&mut net)[0].addons, Ok(3));
    // A zip that is not the one hashed.
    let url = "http://r/omega/zips/script.module.dep/script.module.dep-1.1.0.zip";
    net.0.insert(url.into(), zip(&[("script.module.dep/addon.xml", DEP.as_bytes())]));
    assert!(matches!(m.install(&mut net, &mut store, "plugin.video.test"), Err(Error::Checksum(_))));
    assert!(!store.0.contains_key("addons/plugin.video.test/main.py"));
}

#[test]
fn a_plain_addon_is_not_a_repository() {
    let (mut net, mut store) = (web(), Folder::default());
    let z = zip(&[("plugin.video.test/addon.xml", PLUGIN.as_bytes())]);
    net.0.insert("http://x/p.zip".into(), z);
    let mut m = Manager::default();
    assert_eq!(m.add_repository(&mut net, &mut store, "http://x/p.zip").unwrap_err(), Error::NotRepository("plugin.video.test".into()));
    assert!(matches!(m.add_repository(&mut net, &mut store, "http://x/missing.zip"), Err(Error::Fetch { .. })));
    assert!(store.0.is_empty());
}
