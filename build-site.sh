#!/bin/sh
# build-site.sh - generate the lorevcs.com static site from the README.
# output goes to ./site, ready for: wrangler pages deploy site
set -eu

out=site
rm -rf "$out"
mkdir -p "$out"

# the install script is served verbatim at lorevcs.com/install.sh
cp install.sh "$out/install.sh"

# brand favicon
cp favicon.svg "$out/favicon.svg"

# serve the script as text rather than offering it as a download
cat > "$out/_headers" <<'EOF'
/install.sh
  content-type: text/plain; charset=utf-8
EOF

# the landing page is the README, escaped and dropped into a <pre>, with the
# two urls linkified. same copy, same monospace look.
{
	cat <<'EOF'
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>lore</title>
<meta name="description" content="the latent repository. version control for intent, not code.">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<style>
  html { background: #e7e4dc; }
  body { margin: 0; padding: 2rem 1rem; display: flex; justify-content: center;
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;
    color: #1a1a1a; -webkit-text-size-adjust: 100%; }
  main { background: #fffdf7; border: 2px solid #1a1a1a; box-shadow: 6px 6px 0 #1a1a1a;
    padding: 1.75rem 2rem; max-width: 100%; overflow-x: auto; }
  /* the widest line is ~74 monospace cols and the ascii art can't reflow, so
     scale the type to fit the viewport, capped at 13px on wide screens */
  pre { margin: 0; font: inherit; line-height: 1.5; white-space: pre;
    font-size: clamp(6px, calc((100vw - 4rem) / 46), 13px); }
  a { color: #297e78; text-decoration: underline; }
  a:hover { background: #297e78; color: #fffdf7; text-decoration: none; }
  ::selection { background: #1a1a1a; color: #fffdf7; }
  @media (max-width: 720px) {
    body { padding: 0.6rem; }
    main { padding: 1rem; box-shadow: 4px 4px 0 #1a1a1a; }
  }
</style>
</head>
<body>
<main><pre>
EOF
	sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' \
	    -e 's#https://lorevcs.com/install.sh#<a href="https://lorevcs.com/install.sh">https://lorevcs.com/install.sh</a>#g' \
	    -e 's#https://github.com/lorevcs/lore#<a href="https://github.com/lorevcs/lore">https://github.com/lorevcs/lore</a>#g' \
	    README
	cat <<'EOF'
</pre></main>
</body>
</html>
EOF
} > "$out/index.html"

printf 'built %s/index.html and %s/install.sh\n' "$out" "$out" >&2
