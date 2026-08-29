import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import useBaseUrl from '@docusaurus/useBaseUrl';
import Layout from '@theme/Layout';
import CodeBlock from '@theme/CodeBlock';
import ThemedImage from '@theme/ThemedImage';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
import WebFrame from '@site/src/components/WebFrame';
import Heading from '@theme/Heading';

import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero', styles.heroBanner)}>
      <div className="container">
        {/* Decorative: the <h1> below already says "mecha", and a screen reader
            announcing it twice is noise rather than information. ThemedImage
            rather than a CSS filter because accent-400 and accent-700 are two
            different marks in the brand, not one mark dimmed. */}
        <div className={styles.heroMark} aria-hidden="true">
          <ThemedImage
            alt=""
            sources={{
              light: useBaseUrl('/img/logo-light.svg'),
              dark: useBaseUrl('/img/logo.svg'),
            }}
          />
        </div>
        <Heading as="h1" className="hero__title">
          {siteConfig.title}
        </Heading>
        <p className={clsx('hero__subtitle', styles.kicker)}>
          LOCAL-FIRST AGENT HARNESS · RUST · MIT
        </p>
        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className={styles.buttons}>
          <Link
            className="button button--primary button--lg"
            to="/docs/getting-started/installation">
            Get started
          </Link>
          <Link
            className="button button--secondary button--lg"
            to="/docs/intro">
            What it is
          </Link>
          <Link
            className="button button--secondary button--lg"
            href="https://github.com/ljchang/mecha">
            GitHub
          </Link>
        </div>
      </div>
    </header>
  );
}

// The app, on the front page. A local-first assistant is a hard thing to
// picture from prose — "it runs on your own machine" describes a negative — and
// the surface most people would actually touch it through is the one nothing on
// this site used to show. The frame is the real bundle from `web/` against
// fixtures; `src/components/WebFrame` says why it cannot be a screenshot.
function Surface() {
  return (
    <section className={styles.surface}>
      <div className="container">
        <div className={styles.surfaceHead}>
          <Heading as="h2">And carry it</Heading>
          <p>
            <code>mecha serve</code> puts the same agent behind a web app on your tailnet —
            bound to loopback, fronted by Tailscale, opened by your network identity rather
            than a password. It is where drafts get approved, mail gets read and queues get
            cleared, and it is the only door{' '}
            <Link to="/docs/features/voice">voice</Link> opens through.
          </p>
        </div>
        <WebFrame
          page="home"
          height={700}
          pages={[
            {hash: 'home', label: 'home'},
            {hash: 'chat', label: 'chat'},
            {hash: 'mail', label: 'mail'},
            {hash: 'review', label: 'review'},
            {hash: 'tasks', label: 'tasks'},
            {hash: 'graph/Priya%20Raghavan', label: 'graph'},
          ]}
          caption={
            <>
              Live, not a screenshot — the real app with fixtures behind it. More on{' '}
              <Link to="/docs/features/web">the web surface</Link>.
            </>
          }
        />
      </div>
    </section>
  );
}

function Sample() {
  return (
    <section className={styles.sample}>
      <div className="container">
        <div className="row">
          <div className="col col--6">
            <Heading as="h2">Run it</Heading>
            <p>
              One binary, five front ends — four in a terminal and{' '}
              <Link to="/docs/features/web">one in a browser</Link>.{' '}
              <code>mecha run</code> answers and exits; <code>mecha tui</code> and{' '}
              <code>mecha serve</code> keep the input live, so you can redirect a run
              without stopping it.
            </p>
            <CodeBlock language="bash">
              {`mecha tools                     # no provider needed: lists the surface
mecha run "summarise the notes directory"
mecha tui                       # full screen; steer a run in flight
mecha serve                     # the same agent, on your phone
mecha trigger add briefing --schedule "0 7 * * 1-5" \\
  --prompt "What is on my calendar today?"`}
            </CodeBlock>
          </div>
          <div className="col col--6">
            <Heading as="h2">Connect it</Heading>
            <p>
              An assistant is only as good as what it knows about you. Personal
              context arrives over MCP, so adding a source is configuration
              rather than a code change — and <code>[outbox]</code> means the
              ones that can send stage drafts for you instead.
            </p>
            <CodeBlock language="toml">
              {`[[mcp]]
name = "mail"          # every account behind one surface
command = "mecha-mail"

[[mcp]]
name = "pkg"           # who people are, and what happened when
command = "pkg-mcp"

[outbox]               # staged for review, never sent outright
tools = ["mail__mail_send", "mail__mail_reply"]`}
            </CodeBlock>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title="An agent harness for local models"
      description={siteConfig.tagline as string}>
      <HomepageHeader />
      <main>
        <HomepageFeatures />
        <Surface />
        <Sample />
      </main>
    </Layout>
  );
}
