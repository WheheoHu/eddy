// Feeds an RSSHub instance can make for the page being looked at.
//
// An instance lists what it knows at /api/radar/rules, grouped by site and
// by subdomain ("." for none):
//
//   { "example.com": {
//       "_name": "Example",
//       ".":    [ { "title": "User", "source": ["/user/:id"], "target": "/example/user/:id" } ],
//       "blog": [ { "title": "Posts", "source": ["/", "/page/:n?"], "target": "/example/blog" } ] } }
//
// A rule applies when one of its `source` paths matches the page's; the
// feed is the instance's address followed by `target`, with the names the
// source matched put in. A target that needs a name the page did not give
// (a rule for the site at large, to be filled in by hand) is left out.

// A path and the query after it. The question mark that makes a name
// optional ("/:tab?") is not the one that starts a query ("?v=:id").
function splitQuery(text) {
  const found = /^(.*?)\?([A-Za-z0-9_.~-]+=.*)$/.exec(text.split("#")[0]);
  return found ? [found[1], found[2]] : [text.split("#")[0], ""];
}

// Turns "/user/:id/:tab?" into a pattern for a whole path and the names it
// captures, in order. `:name` is one segment, `:name?` one that may be
// missing, `:name{…}` one matching the expression in braces, `:name*` and
// `*` whatever is left.
function compileSource(source) {
  const [path] = splitQuery(source);
  const names = [];
  let pattern = "";
  for (const segment of path.split("/").filter((s) => s !== "")) {
    const named = /^:([A-Za-z0-9_]+)(\{.+\})?([?*+])?$/.exec(segment);
    if (segment === "*" || (named && (named[3] === "*" || named[3] === "+"))) {
      names.push(named ? named[1] : "*");
      pattern += named && named[3] === "+" ? "/(.+)" : "(?:/(.*))?";
      break;
    }
    if (named) {
      names.push(named[1]);
      const one = named[2] ? named[2].slice(1, -1) : "[^/]+";
      pattern += named[3] === "?" ? `(?:/(${one}))?` : `/(${one})`;
    } else {
      // Plain text; a star inside it stands for anything but a slash.
      pattern += "/" + segment.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*");
    }
  }
  try {
    return { regex: new RegExp(`^${pattern}/?$`), names };
  } catch (e) {
    return null;
  }
}

// What the source's names stand for on this page, or null when the page is
// not one the source describes. A source may also ask for a query value,
// as in "/watch?v=:id".
function matchSource(source, url) {
  const compiled = compileSource(source);
  if (!compiled) return null;
  // Matched as written in the address, so that an escaped slash inside a
  // name stays inside it; what a name stands for is then unescaped.
  const path = url.pathname;
  const found = compiled.regex.exec(path === "" ? "/" : path);
  if (!found) return null;
  const params = {};
  compiled.names.forEach((name, index) => {
    let value = found[index + 1];
    if (value === undefined || value === "") return;
    try {
      value = decodeURIComponent(value);
    } catch (e) {}
    params[name] = value;
  });
  for (const pair of splitQuery(source)[1].split("&").filter((p) => p.includes("="))) {
    const [key, wanted] = pair.split("=");
    const value = url.searchParams.get(key);
    if (value === null) return null;
    if (wanted.startsWith(":")) params[wanted.slice(1)] = value;
    else if (wanted !== value) return null;
  }
  return params;
}

// The target with its names filled in; null when one it cannot do without
// is missing. From the first optional name the page did not give, the rest
// of the path is dropped: later names would land in the wrong places.
function fillTarget(target, params) {
  const [path, query] = splitQuery(target);
  const out = [];
  for (const segment of path.split("/").filter((s) => s !== "")) {
    const named = /^:([A-Za-z0-9_]+)(\{.+\})?([?*+])?$/.exec(segment);
    if (!named) {
      if (segment.includes(":") || segment === "*") return null;
      out.push(segment);
      continue;
    }
    const value = params[named[1]];
    if (value === undefined) {
      if (named[3] === "?" || named[3] === "*") break;
      return null;
    }
    out.push(named[3] ? value : encodeURIComponent(value));
  }
  return "/" + out.join("/") + (query ? "?" + query : "");
}

// The rule lists that may apply to a host: for "blog.example.com", the
// "blog" list of "example.com". "www" and no subdomain are the same place.
function rulesFor(rules, host) {
  const labels = host.toLowerCase().split(".");
  const lists = [];
  for (let from = 0; from <= labels.length - 2; from++) {
    const site = rules[labels.slice(from).join(".")];
    if (!site || typeof site !== "object") continue;
    const sub = labels.slice(0, from).join(".");
    const keys = sub === "" || sub === "www" ? [".", "www"] : [sub];
    for (const key of keys) {
      if (Array.isArray(site[key])) lists.push({ name: site._name || "", list: site[key] });
    }
  }
  return lists;
}

// Every feed the instance at `base` can make for `pageUrl`.
function radarFeeds(rules, pageUrl, base, key) {
  let url;
  try {
    url = new URL(pageUrl);
  } catch (e) {
    return [];
  }
  if (!rules || typeof rules !== "object" || !/^https?:$/.test(url.protocol)) return [];
  const root = String(base || "").replace(/\/+$/, "");
  const feeds = [];
  const seen = new Set();
  for (const { name, list } of rulesFor(rules, url.hostname)) {
    for (const rule of list) {
      if (!rule || typeof rule.target !== "string" || !Array.isArray(rule.source)) continue;
      for (const source of rule.source) {
        const params = typeof source === "string" ? matchSource(source, url) : null;
        if (!params) continue;
        const target = fillTarget(rule.target, params);
        if (!target) continue;
        let feed = root + target;
        if (key) feed += (feed.includes("?") ? "&" : "?") + "key=" + encodeURIComponent(key);
        if (seen.has(feed)) break;
        seen.add(feed);
        const title = rule.title || "";
        feeds.push({ url: feed, title: name && !title.includes(name) ? `${name} · ${title}`.replace(/ · $/, "") : title || name });
        break;
      }
    }
  }
  return feeds.slice(0, 12);
}

// How many rules there are, for the settings page to say.
function countRules(rules) {
  let count = 0;
  for (const site of Object.values(rules || {})) {
    for (const list of Object.values(site || {})) {
      if (Array.isArray(list)) count += list.length;
    }
  }
  return count;
}

// Asks an instance for its rules. Throws with a few words on what went
// wrong: not reached, an HTTP status, an answer that is not a rule list.
async function fetchRules(base, key) {
  const root = String(base || "").replace(/\/+$/, "");
  let response;
  try {
    response = await fetch(`${root}/api/radar/rules${key ? "?key=" + encodeURIComponent(key) : ""}`, {
      credentials: "omit",
      signal: AbortSignal.timeout(20000),
    });
  } catch (e) {
    throw new Error(e?.name === "TimeoutError" ? "No answer after 20 seconds" : "Couldn’t reach the instance");
  }
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  let rules;
  try {
    rules = await response.json();
  } catch (e) {
    throw new Error("The answer is not a list of rules");
  }
  if (!rules || typeof rules !== "object" || Array.isArray(rules)) throw new Error("The answer is not a list of rules");
  return rules;
}

if (typeof module !== "undefined") module.exports = { compileSource, matchSource, fillTarget, radarFeeds, countRules, fetchRules };
