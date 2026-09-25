import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import CodeBlock from '@theme/CodeBlock';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
import WebFrame from '@site/src/components/WebFrame';
import Poster from '@site/src/components/Poster';
import Heading from '@theme/Heading';

import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero', styles.heroBanner)}>
      <div className="container">
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
            Overview
          </Link>
          <Link
            className="button button--secondary button--lg"
            href="https://github.com/ljchang/mecha">
            GitHub
          </Link>
        </div>
      </div>
      {/* The assembled suit, under the pitch rather than beside it: the sheet
          is dense with callouts and needs the full width to be read at all.
          `Poster` says why light and dark get different files. */}
      <div className={clsx('container', styles.heroPoster)}>
        <Poster
          name="assembled"
          cutout
          eager
          alt="Concept schematic of the mecha exosuit: an M-shaped frame on two legs, with the pilot interface slung beneath the upper chassis and its seven modules — perception, memory, language, appraisal, reasoning, learning, motor/toolcall — shown exploded."
        />
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
            <Link to="/docs/features/interfaces/voice">voice</Link> opens through.
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
              <Link to="/docs/features/interfaces/web">the web surface</Link>.
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
              <Link to="/docs/features/interfaces/web">one in a browser</Link>.{' '}
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

[mcp.capabilities]     # other people's words: config says so, no annotation can
untrusted_input = true

[[mcp]]
name = "graph"         # who people are, and what happened when
command = "mecha-graph-mcp"
prefix_tools = false   # its kg_* tools carry their own namespace

[mcp.capabilities]     # the graph holds what mail said, so the same override
untrusted_input = true

[outbox]               # staged for review, never sent outright
tools = [
  "mail__mail_send", "mail__mail_reply",
  # an invitation reaches other people too
  "mail__calendar_create_event",
  "mail__calendar_update_event",
  "mail__calendar_delete_event",
]`}
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
