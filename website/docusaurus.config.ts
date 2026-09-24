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
  // Anchors too, not just links. The default is `warn`, so the eight
  // intra-page links in the privacy policy's overview table were checked by
  // eye rather than by the build — and "verified" meant "I looked". The next
  // one that does not resolve now fails CI.
  onBrokenAnchors: 'throw',

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

  // Features were regrouped into sections on 2026-09-24, which moved every page
  // under /docs/features/<name>, /docs/graph/ and /docs/factory/. Each old URL
  // forwards to its new home so a bookmark or an external link still lands.
  // /docs/features/appraisal is absent on purpose: it is now the appraisal
  // section's explainer, and a real page cannot also be a redirect.
  plugins: [
    [
      '@docusaurus/plugin-client-redirects',
      {
        redirects: [
          {from: '/docs/category/factory', to: '/docs/features/public-surface'},
          {from: '/docs/category/graph', to: '/docs/features/memory/graph'},
          {from: '/docs/factory/artifacts', to: '/docs/features/public-surface/artifacts'},
          {from: '/docs/factory/gallery', to: '/docs/features/public-surface/gallery'},
          {from: '/docs/factory/inbound-queue', to: '/docs/features/public-surface/inbound-queue'},
          {from: '/docs/factory/notebooks', to: '/docs/features/public-surface/notebooks'},
          {from: '/docs/factory/onboarding', to: '/docs/features/public-surface/onboarding'},
          {from: '/docs/factory/overview', to: '/docs/features/public-surface'},
          {from: '/docs/factory/polls', to: '/docs/features/public-surface/polls'},
          {from: '/docs/factory/slides', to: '/docs/features/public-surface/slides'},
          {from: '/docs/features/anticipation', to: '/docs/features/appraisal/anticipation'},
          {from: '/docs/features/appraisal-overview', to: '/docs/features/appraisal'},
          {from: '/docs/features/charter', to: '/docs/features/appraisal/charter'},
          {from: '/docs/features/compaction', to: '/docs/features/models/compaction'},
          {from: '/docs/features/distillation', to: '/docs/features/memory/distillation'},
          {from: '/docs/features/documents', to: '/docs/features/tools/documents'},
          {from: '/docs/features/evaluation', to: '/docs/features/experiments/evaluation'},
          {from: '/docs/features/frontdoor', to: '/docs/features/public-surface/frontdoor'},
          {from: '/docs/features/goals', to: '/docs/features/appraisal/goals'},
          {from: '/docs/features/hooks', to: '/docs/features/security/hooks'},
          {from: '/docs/features/images', to: '/docs/features/interfaces/images'},
          {from: '/docs/features/mail', to: '/docs/features/tools/mail'},
          {from: '/docs/features/outbox', to: '/docs/features/security/outbox'},
          {from: '/docs/features/plan-steps', to: '/docs/features/appraisal/plan-steps'},
          {from: '/docs/features/providers', to: '/docs/features/models/providers'},
          {from: '/docs/features/publishing', to: '/docs/features/public-surface/publishing'},
          {from: '/docs/features/queues', to: '/docs/features/automation/queues'},
          {from: '/docs/features/run-quality', to: '/docs/features/learning/run-quality'},
          {from: '/docs/features/sandbox', to: '/docs/features/security/sandbox'},
          {from: '/docs/features/serving', to: '/docs/features/models/serving'},
          {from: '/docs/features/sessions-and-replay', to: '/docs/features/memory/sessions-and-replay'},
          {from: '/docs/features/skills', to: '/docs/features/learning/skills'},
          {from: '/docs/features/slack', to: '/docs/features/interfaces/slack'},
          {from: '/docs/features/tools-and-mcp', to: '/docs/features/tools'},
          {from: '/docs/features/triggers', to: '/docs/features/automation/triggers'},
          {from: '/docs/features/voice', to: '/docs/features/interfaces/voice'},
          {from: '/docs/features/web', to: '/docs/features/interfaces/web'},
          {from: '/docs/features/work', to: '/docs/features/automation/work'},
          {from: '/docs/features/workflows', to: '/docs/features/automation/workflows'},
          {from: '/docs/graph/architecture', to: '/docs/features/memory/graph/architecture'},
          {from: '/docs/graph/changelog', to: '/docs/features/memory/graph/changelog'},
          {from: '/docs/graph/cli', to: '/docs/features/memory/graph/cli'},
          {from: '/docs/graph/integrations', to: '/docs/features/memory/graph/integrations'},
          {from: '/docs/graph/overview', to: '/docs/features/memory/graph'},
          {from: '/docs/graph/self-improvement', to: '/docs/features/memory/graph/self-improvement'},
          {from: '/docs/graph/tui', to: '/docs/features/memory/graph/tui'},
        ],
      },
    ],
  ],

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
        {
          label: 'Legal',
          position: 'right',
          items: [
            {to: '/privacy', label: 'Privacy policy'},
            {to: '/terms', label: 'Terms of service'},
          ],
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
            {label: 'Evaluation', to: '/docs/features/experiments/evaluation'},
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
            {label: 'Privacy', to: '/privacy'},
            {label: 'Terms', to: '/terms'},
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
