# Torvyn Documentation

This directory contains the source for the Torvyn documentation website. The site has three components:

| Component | Source | Output | URL path |
|---|---|---|---|
| **Landing page** | `landing/index.html` | Copied as-is | `/` |
| **mdBook guides** | `src/` + `book.toml` | `book/` (built) | `/docs/` |
| **Rustdoc API ref** | Crate `///` doc comments | `target/doc/` | `/api/` |

## Building locally

```bash
# From the repo root (torvyn/)
cd docs

# Build mdBook
mdbook build          # output → docs/book/

# Build Rustdoc (from repo root)
cd ..
RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo doc --workspace --no-deps
```

## Assembling the full site

The GitHub Actions workflow (`.github/workflows/docs.yml`) assembles all three components into a `_site/` directory. To replicate locally:

```bash
mkdir -p _site
cp docs/landing/index.html _site/
cp -r docs/landing/assets _site/assets 2>/dev/null || true
cp -r docs/book _site/docs
cp -r target/doc _site/api
cp docs/robots.txt _site/robots.txt
bash docs/scripts/generate-sitemap.sh _site "https://torvyn.github.io/torvyn"
```

Then serve with any static file server:

```bash
python3 -m http.server 8000 --directory _site
```

## Directory layout

```
docs/
├── book.toml              # mdBook configuration
├── book/                  # mdBook build output (git-ignored)
├── landing/
│   ├── index.html         # Standalone landing page
│   └── assets/            # Landing page static assets
├── robots.txt             # Copied into _site at build time
├── scripts/
│   └── generate-sitemap.sh  # Sitemap generator (runs during CI)
├── src/
│   ├── SUMMARY.md         # mdBook table of contents
│   ├── introduction.md    # mdBook landing/intro page
│   ├── getting-started/   # Installation, quickstart, first pipeline
│   ├── concepts/          # Core concept guides
│   ├── tutorials/         # Step-by-step tutorials
│   ├── architecture/      # Architecture overview and design decisions
│   ├── reference/         # CLI, config, WIT, metrics, error codes
│   ├── guides/            # Production deployment, performance tuning
│   ├── examples/          # Complete worked examples
│   ├── use-cases/         # Use case overviews
│   ├── comparisons/       # vs. microservices, containers, etc.
│   ├── contributing/      # Dev setup, coding standards, testing
│   ├── blog/              # Technical blog posts
│   └── internals/         # HLI and LLI design documents
└── theme/
    ├── torvyn.css         # Brand theme overrides
    ├── admonish.css       # Admonition box styles
    └── mermaid-init.js    # Mermaid diagram initialization
```

## Adding or editing content

1. Edit or create markdown files under `src/`.
2. Update `src/SUMMARY.md` if adding a new page — mdBook uses this as its table of contents.
3. Run `mdbook build` to verify the build succeeds with zero warnings.
4. Cross-links to the API reference use the relative path `../api/torvyn_<crate>/index.html` from any mdBook page.

## Deployment

The site deploys automatically via GitHub Actions on every push to `main` that touches `docs/` or `crates/*/src/**`. Pull requests build the site but do not deploy.
