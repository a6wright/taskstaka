#!/usr/bin/env bash
# One-time setup so GitHub Actions can deploy taskstaka.com without a key:
# a deploy service account, a Workload Identity pool + provider trusted for
# this repository only, and the two repo variables the workflow reads.
# Safe to run again. Needs: gcloud (owner on project taskstaka), gh (repo admin).
set -euo pipefail

P=taskstaka
REPO=a6wright/taskstaka
SA=github-deploy@$P.iam.gserviceaccount.com
PN=$(gcloud projects describe "$P" --format='value(projectNumber)')

gcloud services enable iamcredentials.googleapis.com sts.googleapis.com cloudbuild.googleapis.com --project "$P" --quiet

gcloud iam service-accounts describe "$SA" --project "$P" >/dev/null 2>&1 \
  || gcloud iam service-accounts create github-deploy --project "$P" --display-name "GitHub Actions deploy (taskstaka.com)"

# A new service account takes a few seconds to become visible to IAM.
for i in $(seq 1 12); do
  gcloud iam service-accounts describe "$SA" --project "$P" >/dev/null 2>&1 && break
  sleep 5
done
sleep 5

# builds.editor + storage.admin: gcloud builds submit (source upload, logs)
# artifactregistry.writer: push the image; run.admin + serviceAccountUser: deploy
bind() {
  for i in 1 2 3 4 5 6; do
    gcloud projects add-iam-policy-binding "$P" --member "serviceAccount:$SA" --role "$1" --quiet --format=none && return 0
    echo "retrying $1 in 10s"; sleep 10
  done
  return 1
}
for r in roles/cloudbuild.builds.editor roles/storage.admin roles/artifactregistry.writer \
         roles/run.admin roles/iam.serviceAccountUser roles/viewer; do
  bind "$r"
done

gcloud iam workload-identity-pools describe github --project "$P" --location global >/dev/null 2>&1 \
  || gcloud iam workload-identity-pools create github --project "$P" --location global --display-name "GitHub Actions"

gcloud iam workload-identity-pools providers describe github --project "$P" --location global --workload-identity-pool github >/dev/null 2>&1 \
  || gcloud iam workload-identity-pools providers create-oidc github --project "$P" --location global \
       --workload-identity-pool github --display-name GitHub \
       --issuer-uri https://token.actions.githubusercontent.com \
       --attribute-mapping "google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.ref=assertion.ref" \
       --attribute-condition "assertion.repository == '$REPO'"

gcloud iam service-accounts add-iam-policy-binding "$SA" --project "$P" --role roles/iam.workloadIdentityUser \
  --member "principalSet://iam.googleapis.com/projects/$PN/locations/global/workloadIdentityPools/github/attribute.repository/$REPO" \
  --quiet --format=none

gh variable set GCP_WIF_PROVIDER --repo "$REPO" --body "projects/$PN/locations/global/workloadIdentityPools/github/providers/github"
gh variable set GCP_DEPLOY_SA --repo "$REPO" --body "$SA"
gh variable list --repo "$REPO"
echo "done: push to main (or 'gh workflow run deploy-site.yml') now deploys taskstaka.com"
