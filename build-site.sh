#!/bin/sh
# build-site.sh - generate the lorevcs.com static site from the README.
# output goes to ./site, ready for: wrangler pages deploy site
set -eu

out=site
rm -rf "$out"
mkdir -p "$out"

# the install script is served verbatim at lorevcs.com/install.sh
cp install.sh "$out/install.sh"

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
<style>
  html { background: #e7e4dc; }
  body { margin: 0; padding: 2rem 1rem; display: flex; justify-content: center;
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;
    color: #1a1a1a; }
  main { background: #fffdf7; border: 2px solid #1a1a1a; box-shadow: 6px 6px 0 #1a1a1a;
    padding: 1.75rem 2rem; max-width: 100%; overflow-x: auto; }
  pre { margin: 0; font: inherit; font-size: 13px; line-height: 1.45; white-space: pre; }
  a { color: #297e78; text-decoration: underline; }
  a:hover { background: #297e78; color: #fffdf7; text-decoration: none; }
  ::selection { background: #1a1a1a; color: #fffdf7; }
  @media (max-width: 640px) { pre { font-size: 10px; } main { padding: 1rem; } }
</style>
</head>
<body>
<main><pre>
EOF
	sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' \
	    -e 's#https://lorevcs.com/install.sh#<a href="https://lorevcs.com/install.sh">https://lorevcs.com/install.sh</a>#g' \
	    -e 's#https://github.com/raymondjacobson/lore#<a href="https://github.com/raymondjacobson/lore">https://github.com/raymondjacobson/lore</a>#g' \
	    README
	cat <<'EOF'
</pre></main>
</body>
</html>
EOF
} > "$out/index.html"

printf 'built %s/index.html and %s/install.sh\n' "$out" "$out" >&2
