import { addComment as addGithubComment } from './comments.js';
import { GhCommandClient } from './gh-client.js';
import { listIssues as listGithubIssues, mirrorTaskIssue } from './issues.js';
import { bootstrapLabels, listVerifiedLabels as listGithubLabels } from './labels.js';
import { ensureMilestone as ensureGithubMilestone, listMilestones as listGithubMilestones, } from './milestones.js';
import { createPullRequest as createGithubPullRequest, detectDefaultBranch as detectGithubDefaultBranch, viewPullRequest as viewGithubPullRequest, } from './pull-requests.js';
export class GithubCliAdapter {
    context;
    constructor(process, options = {}) {
        this.context = {
            client: new GhCommandClient(process),
            dryRun: options.dryRun ?? false,
            githubBootstrap: options.githubBootstrap ?? false,
            log: options.log ??
                ((message) => {
                    console.warn(message);
                }),
        };
    }
    async detectDefaultBranch(cwd) {
        return detectGithubDefaultBranch(this.context, cwd);
    }
    async listVerifiedLabels(cwd) {
        return listGithubLabels(this.context, cwd);
    }
    async bootstrapLabels(cwd, labels, options = {}) {
        return bootstrapLabels(this.context, cwd, labels, options);
    }
    async listMilestones(cwd) {
        return listGithubMilestones(this.context, cwd);
    }
    async ensureMilestone(cwd, title, options = {}) {
        return ensureGithubMilestone(this.context, cwd, title, options);
    }
    async listIssues(cwd) {
        return listGithubIssues(this.context, cwd);
    }
    async mirrorTaskIssue(cwd, request, options = {}) {
        return mirrorTaskIssue(this.context, cwd, request, options);
    }
    async createPullRequest(request, options = {}) {
        return createGithubPullRequest(this.context, request, options);
    }
    async viewPullRequest(cwd, number) {
        return viewGithubPullRequest(this.context, cwd, number);
    }
    async addComment(cwd, request, options = {}) {
        return addGithubComment(this.context, cwd, request, options);
    }
}
