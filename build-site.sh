#!/bin/sh
# build-site.sh - generate the lorevcs.com static site from the README.
# output goes to ./site, ready for: wrangler pages deploy site
set -eu

out=site
rm -rf "$out"
mkdir -p "$out"

# the install script is served verbatim at lorevcs.com/install.sh
cp install.sh "$out/install.sh"

# brand favicon and social-share (unfurl) image
cp favicon.svg "$out/favicon.svg"
cp og.png "$out/og.png"

# serve the script as text rather than offering it as a download
cat > "$out/_headers" <<'EOF'
/*
  X-Content-Type-Options: nosniff
  Referrer-Policy: strict-origin-when-cross-origin
/install.sh
  content-type: text/plain; charset=utf-8
EOF

# let crawlers in and point them at the sitemap
cat > "$out/robots.txt" <<'EOF'
User-agent: *
Allow: /
Sitemap: https://lorevcs.com/sitemap.xml
EOF

cat > "$out/sitemap.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://lorevcs.com/</loc><changefreq>weekly</changefreq></url>
</urlset>
EOF

# the landing page is the README. the ascii logo can't reflow, so it scales to
# fit; the prose wraps at a readable size on narrow screens. same copy.
split=$(awk '/^$/{print NR; exit}' README)
{
	cat <<'EOF'
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>lore - version control for intent, not code</title>
<meta name="description" content="lore is version control for intent, not code: commit the prompts and decisions behind a codebase, then materialize it on demand. A tiny, git-like CLI.">
<link rel="canonical" href="https://lorevcs.com/">
<meta name="theme-color" content="#41b8a8">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<meta property="og:type" content="website">
<meta property="og:locale" content="en_US">
<meta property="og:site_name" content="lore">
<meta property="og:title" content="lore - version control for intent, not code">
<meta property="og:description" content="Commit the prompts, notes, and decisions behind a codebase, then materialize it on demand. A tiny, git-like CLI.">
<meta property="og:url" content="https://lorevcs.com/">
<meta property="og:image" content="https://lorevcs.com/og.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="lore - version control for intent, not code">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="lore - version control for intent, not code">
<meta name="twitter:description" content="Commit the prompts, notes, and decisions behind a codebase, then materialize it on demand.">
<meta name="twitter:image" content="https://lorevcs.com/og.png">
<meta name="twitter:image:alt" content="lore - version control for intent, not code">
<script type="application/ld+json">
{"@context":"https://schema.org","@graph":[{"@type":"WebSite","@id":"https://lorevcs.com/#website","name":"lore","alternateName":["lore vcs","the latent repository"],"url":"https://lorevcs.com/","description":"Version control for intent, not code."},{"@type":"SoftwareApplication","name":"lore","alternateName":"the latent repository","applicationCategory":"DeveloperApplication","operatingSystem":"macOS, Linux","description":"Version control for intent, not code. Commit the prompts, notes, and decisions behind a codebase, then materialize it on demand with an AI agent.","url":"https://lorevcs.com/","downloadUrl":"https://lorevcs.com/install.sh","softwareVersion":"0.1","license":"https://opensource.org/licenses/MIT","codeRepository":"https://github.com/lorevcs/lore","keywords":"version control for intent, version control for prompts, latent repository, track intent not code, materialize, ai agents, prompt engineering, git, rust, cli","sameAs":["https://github.com/lorevcs/lore"],"author":{"@type":"Person","name":"Raymond Jacobson"},"offers":{"@type":"Offer","price":"0","priceCurrency":"USD"}}]}
</script>
<style>
  html { background: #e7e4dc; }
  body { margin: 0; padding: 2rem 1rem; display: flex; justify-content: center;
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;
    color: #1a1a1a; -webkit-text-size-adjust: 100%; }
  main { background: #fffdf7; border: 2px solid #1a1a1a; box-shadow: 6px 6px 0 #1a1a1a;
    padding: 1.75rem 2rem; max-width: 100%; }
  /* the ascii logo can't reflow: scale it to fit, capped at 13px on wide screens */
  .art { margin: 0 auto 1.25rem; width: fit-content; white-space: pre; overflow-x: auto;
    font-size: clamp(6px, calc((100vw - 4rem) / 46), 13px); line-height: 1.2; }
  /* prose stays readable and wraps on narrow screens */
  .body { margin: 0; font: inherit; font-size: 13px; line-height: 1.55;
    white-space: pre-wrap; overflow-wrap: anywhere; max-width: 76ch; }
  a { color: #297e78; text-decoration: underline; }
  a:hover { background: #297e78; color: #fffdf7; text-decoration: none; }
  ::selection { background: #1a1a1a; color: #fffdf7; }
  /* a real heading for crawlers and screen readers; the visible one is ascii */
  .sr-only { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0;
    overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border: 0; }
  @media (max-width: 720px) {
    body { padding: 0.6rem; }
    main { padding: 1.1rem; box-shadow: 4px 4px 0 #1a1a1a; }
  }
</style>
</head>
<body>
<main>
<h1 class="sr-only">lore: version control for intent, not code</h1>
<pre class="art">
EOF
	head -n "$((split - 1))" README |
		sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
	cat <<'EOF'
</pre>
<pre class="body">
EOF
	tail -n "+$((split + 1))" README |
		sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' \
			-e 's#\(https://[^ ]*\)#<a href="\1">\1</a>#g'
	cat <<'EOF'
</pre>
</main>
</body>
</html>
EOF
} > "$out/index.html"

printf 'built %s with og image, robots, and sitemap\n' "$out" >&2
