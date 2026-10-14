#!/usr/bin/env python3
"""Select immutable packages from a fully qualified same-commit main push run."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import urlencode

from release_assets import TARGETS

WORKFLOW = '.github/workflows/native.yml'
ROWS = ('macos-arm64', 'macos-x86_64', 'linux-x86_64', 'linux-arm64', 'windows-x86_64')
CHECK_JOB = 'Check'


def api(endpoint):
    result = subprocess.run(
        ['gh', 'api', '-H', 'Accept: application/vnd.github+json',
         '-H', 'X-GitHub-Api-Version: 2022-11-28', endpoint],
        check=True, capture_output=True, text=True,
    )
    return json.loads(result.stdout)


def pages(endpoint, key):
    """Never qualify against a truncated first page of jobs or artifacts."""
    separator = '&' if '?' in endpoint else '?'
    result = []
    for page in range(1, 1001):
        response = api(f'{endpoint}{separator}per_page=100&page={page}')
        result.extend(response[key])
        if len(response[key]) < 100:
            if len(result) != response['total_count']:
                raise ValueError(f'Incomplete or changing API listing: {key}')
            return result
    raise ValueError(f'API listing exceeds qualification limit: {key}')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def timestamp(value):
    return datetime.fromisoformat(value.replace('Z', '+00:00'))


def qualify(repository, repository_id, sha):
    base = f'repos/{repository}'
    workflow = api(f'{base}/actions/workflows/native.yml')
    require(workflow['path'] == WORKFLOW and workflow['state'] == 'active',
            'Native qualification workflow is not active at its expected path')
    query = urlencode({'branch': 'main', 'event': 'push', 'head_sha': sha})
    runs = pages(f'{base}/actions/workflows/{workflow["id"]}/runs?{query}', 'workflow_runs')
    require(runs, 'No native main push run exists for the tagged commit; run native CI first')
    # Never fall back to an older green run when the newest run failed or is pending.
    run = api(f'{base}/actions/runs/{max(item["id"] for item in runs)}')
    run_id = run['id']
    attempt = run['run_attempt']
    require(run['workflow_id'] == workflow['id'] and run['path'] == WORKFLOW,
            'Unexpected source workflow')
    require(run['event'] == 'push' and run['head_branch'] == 'main' and run['head_sha'] == sha,
            'Qualification must be a main push for the exact tagged commit')
    require(run['repository']['id'] == repository_id
            and run['head_repository']['id'] == repository_id,
            'Qualification must originate in this repository, not a fork')
    require(run['status'] == 'completed' and run['conclusion'] == 'success',
            'Newest same-commit native run has not completed successfully')

    jobs = pages(f'{base}/actions/runs/{run_id}/attempts/{attempt}/jobs', 'jobs')
    expected = {CHECK_JOB} | {f'Build / {row}' for row in ROWS}
    require(len(jobs) == len(expected) and {job['name'] for job in jobs} == expected,
            'Require the check job and all five build rows in the same run attempt; '
            'partial reruns do not qualify, rerun all jobs instead')
    for job in jobs:
        require(job['run_id'] == run_id and job['head_sha'] == sha
                and job['run_attempt'] == attempt
                and job['status'] == 'completed' and job['conclusion'] == 'success',
                f'Unqualified native job: {job["name"]}')
        # A successful job with skipped acceptance steps is not qualification.
        require(job['steps'] and all(step['status'] == 'completed'
                                    and step['conclusion'] == 'success'
                                    for step in job['steps']),
                f'Incomplete or skipped steps in native job: {job["name"]}')
    by_name = {job['name']: job for job in jobs}
    artifacts = pages(f'{base}/actions/runs/{run_id}/artifacts', 'artifacts')
    packages = [artifact for artifact in artifacts if artifact['name'].startswith('ev-')]
    require(len(packages) == len(TARGETS)
            and {item['name'] for item in packages} == {'ev-' + target for target in TARGETS},
            'Require exactly one immutable package for each of the five native targets')
    artifact_ids = []
    for row, target in zip(ROWS, TARGETS, strict=True):
        artifact = next(item for item in packages if item['name'] == 'ev-' + target)
        provenance = artifact['workflow_run']
        require(provenance['id'] == run_id and provenance['head_sha'] == sha
                and provenance['head_branch'] == 'main'
                and provenance['repository_id'] == repository_id
                and provenance['head_repository_id'] == repository_id,
                f'Unexpected artifact provenance: {target}')
        require(artifact['expired'] is False
                and timestamp(artifact['expires_at']) > datetime.now(timezone.utc)
                and 0 < artifact['size_in_bytes'] <= 128 * 1024 * 1024
                and re.fullmatch(r'sha256:[0-9a-f]{64}', artifact['digest'] or ''),
                f'Missing, expired, oversized, or undigested package: {target}')
        build = by_name[f'Build / {row}']
        require(timestamp(build['started_at']) <= timestamp(artifact['created_at'])
                <= timestamp(build['completed_at']),
                f'Package must be uploaded by its build job in this attempt: {target}')
        artifact_ids.append(artifact['id'])

    # Detect a rerun or state change during the paginated qualification reads.
    current = api(f'{base}/actions/runs/{run_id}')
    require(current['run_attempt'] == attempt and current['status'] == 'completed'
            and current['conclusion'] == 'success', 'Native run changed during qualification')
    return {'run-id': str(run_id), 'run-attempt': str(attempt),
            'artifact-ids': ','.join(str(value) for value in artifact_ids)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected-run')
    parser.add_argument('--expected-attempt')
    parser.add_argument('--expected-artifacts')
    args = parser.parse_args()
    repository = os.environ['GITHUB_REPOSITORY']
    sha = os.environ['GITHUB_SHA']
    require(re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository),
            'Invalid repository identity')
    require(re.fullmatch(r'[0-9a-f]{40}', sha), 'Expected a full commit SHA')
    require(os.environ['GITHUB_EVENT_NAME'] == 'push'
            and os.environ['GITHUB_REF'].startswith('refs/tags/v'),
            'Release qualification is only available to version tag pushes')
    selection = qualify(repository, int(os.environ['GITHUB_REPOSITORY_ID']), sha)
    for key, expected in (('run-id', args.expected_run), ('run-attempt', args.expected_attempt),
                          ('artifact-ids', args.expected_artifacts)):
        require(expected is None or selection[key] == expected,
                f'Native qualification changed before publication: {key}')
    if 'GITHUB_OUTPUT' in os.environ:
        with Path(os.environ['GITHUB_OUTPUT']).open('a', encoding='utf-8') as output:
            for key, value in selection.items():
                output.write(f'{key}={value}\n')
    print(json.dumps({'sha': sha, **selection}, sort_keys=True))


if __name__ == '__main__':
    main()
