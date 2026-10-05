#!/usr/bin/env bash
set -u

image=${1:?usage: image_metadata.sh IMAGE}
echo "application_git_commit=$(git rev-parse HEAD 2>/dev/null || echo unavailable)"
echo "image=$image"
if docker image inspect "$image" >/dev/null 2>&1; then
    docker image inspect "$image" --format 'image_id={{.Id}}'
    docker image inspect "$image" --format 'created={{.Created}}'
    docker image inspect "$image" --format 'repo_digests={{join .RepoDigests ","}}'
else
    echo "image_id=unavailable"
    echo "created=unavailable"
    echo "repo_digests=unavailable"
    exit 1
fi
