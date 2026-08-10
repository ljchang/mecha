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
    title: 'Your model, on your machine',
    to: '/docs/features/providers',
    description: (
      <>
        Built for local open-weight models first — llama-server, vLLM, Ollama —
        with Anthropic there when you want a ceiling to measure against. Your
        mail and your calendar never have to leave the house to be useful.
      </>
    ),
  },
  {
    title: 'Your context, connected',
    to: '/docs/features/mail',
    description: (
      <>
        Mail and calendar behind one surface, a personalized knowledge graph for
        who people are and what happened when, and anything else you can expose
        over MCP. An assistant is only as good as what it knows about you.
      </>
    ),
  },
  {
    title: 'Built for the lethal trifecta',
    to: '/docs/features/security',
    description: (
      <>
        A personal assistant holds private data, reads other people&apos;s words,
        and can send — all three, by definition. So the interlock is structural:
        tools declare capabilities, the conversation carries the taint, and a
        send is refused before the human is ever asked.
      </>
    ),
  },
  {
    title: 'Nothing leaves without you',
    to: '/docs/features/outbox',
    description: (
      <>
        Name a tool in the outbox and its calls are staged as drafts instead of
        executed. Overnight inbox triage leaves you a review queue rather than
        sent mail — and it needs no write permission at all, because staging
        executes nothing.
      </>
    ),
  },
  {
    title: 'A public surface, both directions',
    to: '/docs/factory/overview',
    description: (
      <>
        What the agent makes becomes a durable, versioned URL. What people need
        from you arrives as a typed request rather than free-form prose, so a
        stranger&apos;s words never reach a privileged run.
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
