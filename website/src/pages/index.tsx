import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import useBaseUrl from '@docusaurus/useBaseUrl';
import Layout from '@theme/Layout';
import CodeBlock from '@theme/CodeBlock';
import ThemedImage from '@theme/ThemedImage';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
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
          AGENT HARNESS · RUST · MIT
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

function Sample() {
  return (
    <section className={styles.sample}>
      <div className="container">
        <div className="row">
          <div className="col col--6">
            <Heading as="h2">Run it</Heading>
            <p>
              One binary, four front ends. <code>mecha run</code> answers and
              exits; <code>mecha tui</code> keeps the input line live, so you can
              redirect a run without stopping it.
            </p>
            <CodeBlock language="bash">
              {`mecha tools                     # no provider needed: lists the surface
mecha run "summarise the notes directory"
mecha tui                       # full screen; steer a run in flight
mecha trigger add briefing --cron "0 7 * * *" \\
  --prompt "What is on my calendar today?"`}
            </CodeBlock>
          </div>
          <div className="col col--6">
            <Heading as="h2">Embed it</Heading>
            <p>
              <code>mecha-core</code> is a plain Rust library. Implement{' '}
              <code>Tool</code> to add a tool, <code>Provider</code> to add a
              backend, <code>Approver</code> to decide what needs permission.
            </p>
            <CodeBlock language="toml">
              {`[dependencies]
mecha-core = { git = "https://github.com/ljchang/mecha" }

# The loop never learns which provider is behind it,
# or where a tool came from. Both are trait objects.`}
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
      title="A standalone agent harness"
      description={siteConfig.tagline as string}>
      <HomepageHeader />
      <main>
        <HomepageFeatures />
        <Sample />
      </main>
    </Layout>
  );
}
