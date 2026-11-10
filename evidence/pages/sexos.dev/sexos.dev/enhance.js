// Added to the original sexos.dev: a live build log and a ticker that carries the newest commit.
(function () {
  var last; try { last = Number(localStorage.getItem("sexos.lastVisit")) || 0; localStorage.setItem("sexos.lastVisit", String(Date.now())); } catch (e) { last = 0; }
  var day = function (iso) { return new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" }); };
  function show(list) {
    var ol = document.getElementById("bl-list");
    list.slice(0, 6).forEach(function (c) {
      var li = document.createElement("li"), a = document.createElement("a"), code = document.createElement("code"), t = document.createElement("time");
      code.textContent = c.sha; a.href = c.url; a.appendChild(code); a.appendChild(document.createTextNode(c.message));
      t.dateTime = c.date; t.textContent = day(c.date);
      if (last && Date.parse(c.date) > last) li.className = "new";
      li.appendChild(a); li.appendChild(t); ol.appendChild(li);
    });
    if (ol.children.length) document.getElementById("buildlog").hidden = false;
    var ticker = document.getElementById("ticker-text");
    if (ticker && list[0]) ticker.insertBefore(document.createTextNode("LATEST: " + list[0].message.toUpperCase() + "  ◆  "), ticker.firstChild);
  }
  fetch("https://data.narwallagram.com/network.json", { cache: "no-cache" }).then(function (r) { if (!r.ok) throw 0; return r.json(); }).then(function (f) {
    show((f.commits || []).filter(function (c) { return c.repo === "sexos"; }));
  }).catch(function () {
    fetch("feed.json").then(function (r) { return r.json(); }).then(function (f) { show((f.commits || []).filter(function (c) { return c.repo === "sexos"; })); }).catch(function () {});
  });
})();
