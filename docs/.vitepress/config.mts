import { defineConfig } from "vitepress";

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "Aperio",
  description: "Screamingly fast, ultra-lightweight search engine.",
  sitemap: { hostname: "https://aperiosearch.com" },
  head: [
    ["link", { rel: "icon", href: "/favicon.ico", sizes: "any" }],
    [
      "link",
      {
        rel: "icon",
        href: "/favicon-32x32.png",
        type: "image/png",
        sizes: "32x32",
      },
    ],
    [
      "link",
      {
        rel: "icon",
        href: "/favicon-16x16.png",
        type: "image/png",
        sizes: "16x16",
      },
    ],
    [
      "link",
      {
        rel: "apple-touch-icon",
        href: "/apple-touch-icon.png",
        sizes: "180x180",
      },
    ],
  ],
  themeConfig: {
    // https://vitepress.dev/reference/default-theme-config
    nav: [
      { text: "Home", link: "/" },
      { text: "Docs", link: "/quickstart" },
      { text: "About", link: "/about" },
    ],
    editLink: {
      pattern: "https://github.com/aperio-search/aperio/edit/main/docs/:path",
    },
    sidebar: [
      {
        text: "Getting Started",
        items: [{ text: "Quickstart", link: "/quickstart" }],
      },
      {
        text: "Guides",
        items: [
          {
            text: "Collections Management",
            items: [
              { text: "Creating Collections", link: "/creating-collections" },
              { text: "Listing Collections", link: "/listing-collections" },
              {
                text: "Viewing Collection Metadata",
                link: "/viewing-collection-metadata",
              },
              { text: "Deleting Collections", link: "/deleting-collections" },
            ],
          },
          {
            text: "Items Management",
            items: [
              { text: "Inserting Items", link: "/inserting-items" },
              { text: "Bulk Ingestion", link: "/bulk-ingestion" },
              { text: "Deleting Items", link: "/deleting-items" },
            ],
          },
          {
            text: "Searching Documents",
            link: "/searching-documents",
          },
          { text: "Suggesting Terms", link: "/suggesting-terms" },
          { text: "Deploying", link: "/deploying" },
          { text: "Import & Export", link: "/import-export" },
          { text: "Queue", link: "/queue" },
          {
            text: "Configuration",
            items: [
              { text: "Configuration File", link: "/configuration" },
              {
                text: "Environment Variables",
                link: "/environment-variables",
              },
            ],
          },
          { text: "Performance", link: "/performance" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "API Reference", link: "/api-reference" },
          { text: "Error Handling", link: "/error-handling" },
          { text: "Status", link: "/status" },
          { text: "Architecture", link: "/architecture" },
        ],
      },
      { text: "About", link: "/about" },
    ],
    search: {
      provider: "local",
    },
    footer: {
      copyright:
        '© 2026 <a href="https://github.com/andresribeiro">André Ribeiro</a> · <a href="https://obelinf.com">Obelinf.com</a>',
    },
    socialLinks: [
      { icon: "github", link: "https://github.com/aperio-search/aperio" },
      { icon: "x", link: "https://x.com/aperiosearch" },
    ],
  },
});
