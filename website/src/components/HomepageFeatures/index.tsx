import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  to: string;
  description: ReactNode;
};

const FeatureList: FeatureItem[] = [
  {
    title: 'One loop, any backend',
    to: '/docs/features/providers',
    description: (
      <>
        Anthropic over raw HTTP, or anything OpenAI-compatible — llama-server,
        vLLM, Ollama. Transient failures are classified and retried; a retry
        never duplicates work already shown or acted on.
      </>
    ),
  },
  {
    title: 'Injection resistance, enforced structurally',
    to: '/docs/features/security',
    description: (
      <>
        Tools declare capabilities, and the loop refuses to send anything
        outward once private and untrusted data are both in the conversation.
        It sits ahead of the human approver, because a person clicking yes is
        what an injection is trying to engineer.
      </>
    ),
  },
  {
    title: 'A path jail, not a prompt',
    to: '/docs/features/sandbox',
    description: (
      <>
        Every model-supplied path is canonicalized and proven to be inside the
        workspace before any filesystem call. Confinement for <code>shell</code>{' '}
        comes from bubblewrap or docker, and a sandbox that does not work stops
        the run rather than falling back.
      </>
    ),
  },
  {
    title: 'Tools, subagents, and MCP',
    to: '/docs/features/tools-and-mcp',
    description: (
      <>
        Built-in file and shell tools, plus any MCP server over stdio. Servers
        get a cleared environment and a named allowlist, so a third-party server
        cannot quietly read your provider keys.
      </>
    ),
  },
  {
    title: 'Rules that keep earning their seat',
    to: '/docs/features/learning',
    description: (
      <>
        mecha mines the moments you stepped in and consolidates them into rules.
        A validation ledger measures each one, and a rule that accumulates
        attributed regressions is proposed for retirement — measured harm, not a
        model&apos;s opinion.
      </>
    ),
  },
  {
    title: 'Graded on the trace, not the claim',
    to: '/docs/features/evaluation',
    description: (
      <>
        The eval rig checks the tool calls first and the prose second, runs a
        verify command for ground truth, and reports pass^k beside pass@k.
        Everything a model says about its own work is hearsay.
      </>
    ),
  },
];

function Feature({title, description, to}: FeatureItem) {
  return (
    <div className={clsx('col col--4')}>
      <div className={styles.card}>
        <Heading as="h3" className={styles.cardTitle}>
          <Link to={to}>{title}</Link>
        </Heading>
        <p className={styles.cardBody}>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
