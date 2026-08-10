import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'mecha',
  tagline: 'Give a local open-weight model your context, your permissions, and a safe way to reach the world',
  // The .ico rather than the .svg: it is what a bare request for /favicon.ico
  // gets, and that request happens whether or not the <link> below is honoured.
  // Both are generated from brand/favicon.svg by scripts/build-brand-assets.py.
  favicon: 'img/favicon.ico',

  // These are absolute paths rather than baseUrl-relative ones, because
  // Docusaurus does not rewrite headTags attributes. They track `baseUrl`
  // below by hand, which is only tolerable because it is now `/`.
  headTags: [
    {
      tagName: 'link',
      attributes: {rel: 'icon', type: 'image/svg+xml', href: '/img/favicon.svg'},
    },
    {
      tagName: 'link',
      attributes: {rel: 'apple-touch-icon', href: '/img/apple-touch-icon.png'},
    },
  ],

  future: {
    v4: true,
  },

  // Served from GitHub Pages under a custom domain, which is why `baseUrl` is
  // `/` rather than `/mecha/`. The custom domain is asserted by `static/CNAME`
  // — a file in the *artifact*, since the deploy goes through
  // `actions/deploy-pages` and nothing else writes one.
  //
  // Docs stay on Pages deliberately: the factory box serves three origins under
  // deliberately strict policies, and a docs site is exactly the "arbitrary
  // hosting" it declines to be. Nothing here needs the box, so nothing here
  // costs it.
  url: 'https://docs.mecha-factory.ai',
  baseUrl: '/',

  organizationName: 'ljchang',
  projectName: 'mecha',
  trailingSlash: false,

  // A broken link is a docs bug, and CI is the right place to find it.
  onBrokenLinks: 'throw',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },
  themes: ['@docusaurus/theme-mermaid'],

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          routeBasePath: 'docs',
          editUrl: 'https://github.com/ljchang/mecha/tree/main/website/',
        },
        // No blog: this site documents a tool, and an empty blog is a dead link
        // in the navbar rather than a feature.
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    // The social card is a PNG, not the SVG it is generated from: Twitter,
    // Slack and iMessage will not render an SVG preview, so the vector would
    // silently produce no card at all.
    image: 'img/og-card.png',
    // Dark-first, and it does not follow the OS. The palette is built on the
    // void ground; the light theme is a courtesy and reads as the alternate.
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: false,
    },
    navbar: {
      title: 'mecha',
      logo: {
        alt: 'mecha',
        // Two files rather than one filtered file. accent-400 is a dark-ground
        // colour and brand.md swaps it to accent-700 on a light one; supplying
        // both also means Docusaurus renders its themed pair, which is what it
        // does regardless — with only `src` it emits the dark variant and then
        // hides it in the light theme, so the mark disappears.
        src: 'img/logo-light.svg',
        srcDark: 'img/logo.svg',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Documentation',
        },
        {
          to: '/docs/changelog',
          label: 'Changelog',
          position: 'left',
        },
        {
          href: 'https://github.com/ljchang/mecha',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      logo: {
        alt: 'mecha',
        src: 'img/logo-mono.svg',
        href: 'https://github.com/ljchang/mecha',
        width: 42,
      },
      links: [
        {
          title: 'Documentation',
          items: [
            {label: 'Overview', to: '/docs/intro'},
            {label: 'Getting started', to: '/docs/getting-started/installation'},
            {label: 'Configuration', to: '/docs/reference/configuration'},
          ],
        },
        {
          title: 'Concepts',
          items: [
            {label: 'Security model', to: '/docs/features/security'},
            {label: 'Learning', to: '/docs/features/learning'},
            {label: 'Evaluation', to: '/docs/features/evaluation'},
          ],
        },
        {
          title: 'Project',
          items: [
            {label: 'GitHub', href: 'https://github.com/ljchang/mecha'},
            {label: 'Changelog', to: '/docs/changelog'},
            {
              label: 'License (MIT)',
              href: 'https://github.com/ljchang/mecha/blob/main/LICENSE',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Luke Chang. MIT licensed. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      // palenight, not dracula: its ground (#292d3e) sits inside the void→surface
      // range and its accents are violet, so a code block reads as part of the
      // page. Dracula's green and pink are a second and third hue, and the brand
      // has exactly one.
      darkTheme: prismThemes.palenight,
      additionalLanguages: ['rust', 'toml', 'bash', 'json'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
