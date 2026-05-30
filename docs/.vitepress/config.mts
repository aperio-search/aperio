import { defineConfig } from "vitepress";

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "Aperio",
  description: "Screamingly fast, ultra-lean search engine.",
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
        text: "Examples",
        items: [
          { text: "Quickstart", link: "/quickstart" },
          { text: "Creating Collections", link: "/creating-collections" },
          { text: "Inserting Items", link: "/inserting-items" },
          { text: "Deleting Items", link: "/deleting-items" },
          { text: "Search", link: "/search" },
          { text: "Autocomplete", link: "/autocomplete" },
          { text: "Deleting Collections", link: "/deleting-collections" },
          { text: "Listing Collections", link: "/listing-collections" },
          { text: "Collection Metadata", link: "/collection-metadata" },
          { text: "Performance", link: "/performance" },
          { text: "Deploying", link: "/deploying" },
          { text: "Configuration", link: "/configuration" },
          { text: "About", link: "/about" },
        ],
      },
    ],
    search: {
      provider: "local",
    },
    footer: {
      copyright:
        '© 2026 <a href="https://github.com/andresribeiro">André Ribeiro</a>',
    },
    socialLinks: [
      { icon: "github", link: "https://github.com/aperio-search/aperio" },
      { icon: "x", link: "https://x.com/aperiosearch" },
    ],
  },
});
