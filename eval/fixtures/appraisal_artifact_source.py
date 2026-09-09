#!/usr/bin/env python3
"""Appraisal tasks with the output file explicit in every task prompt.

The v2 privacy prompts named their artifact only in the workspace README. Keep
that registered design intact; this new source removes the ambiguity without
changing inputs or oracle answers.
"""
from appraisal_v2_source import oracle

original_tasks = oracle.tasks


def tasks():
    result = original_tasks()
    for task in result:
        task['prompt'] += ' Write the requested JSON object to answer.json in the workspace.'
    return result


oracle.tasks = tasks

if __name__ == '__main__':
    oracle.main()
