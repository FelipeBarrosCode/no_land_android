export type FeedbackEnrollmentStatus = "not_enrolled" | "enrolling" | "enrolled" | "withdrawn" | "deletion_pending" | "deleted" | "unavailable";

export interface FeedbackPendingControl {
  kind: "consent" | "withdrawal" | "deletion";
  requestId: string;
  expectedEpoch: number;
  enabled: boolean;
  optionalContextEnabled: boolean;
  preserveSessions: boolean;
}

export interface FeedbackProgramState {
  schemaVersion: 1;
  enabled: boolean;
  enrollmentStatus: FeedbackEnrollmentStatus;
  installationId: string | null;
  userId: string | null;
  credentialRef: string | null;
  deletionCodeRef: string | null;
  consentEpoch: number;
  privacyVersion: string | null;
  termsVersion: string | null;
  consentedAt: string | null;
  optionalContextEnabled: boolean;
  pendingControlAction: FeedbackPendingControl | null;
}

export interface FeedbackProgramStatus extends FeedbackProgramState {
  configured: boolean;
  supported: boolean;
  statusMessage: string | null;
  pendingSessions: number;
  jitterCapability?: string;
  frameEvidence?: string;
  receiptId?: string | null;
  deletionStatus?: string | null;
}

export interface FeedbackOptionalProfile {
  country?: string;
  region?: string;
  connectionCategory?: string;
  bandwidthTier?: string;
}

export interface FeedbackEnrollmentRequest {
  privacyVersion: string;
  termsVersion: string;
  participationAccepted: true;
  optionalContextEnabled: boolean;
  optionalProfile: FeedbackOptionalProfile;
}

export interface FeedbackProgramDocument {
  schemaVersion: 1;
  available: boolean;
  collectionPaused: boolean;
  privacyVersion: string;
  termsVersion: string;
  privacyUrl: string;
  termsUrl: string;
  deletionUrl: string;
  contact: string;
  disclosure: string;
  metrics: unknown[];
}

export interface FeedbackJitter {
  meanMs: number;
  minMs: number;
  maxMs: number;
  sampleCount: number;
  contributingSessions: number;
  highSampleRatio: number;
  severeSampleRatio: number;
  metricVersion: string;
  scope: string;
  nominalReportIntervalMs?: number;
  completeness?: string;
  thresholdVersion?: string;
  highThresholdMs?: number;
  severeThresholdMs?: number;
}

export interface FeedbackJitterGroup extends FeedbackJitter {
  routeMode: "direct" | "relay" | "mixed";
}

export interface FeedbackMachineSummary {
  providerMachineId: string;
  reliability: {
    source: "no_land";
    ratio: number | null;
    eligibleSessions: number;
    successfulSessions: number;
    failedSessions: number;
    basis: string;
    scope: string;
    firstObservedAt?: string | null;
    lastObservedAt?: string | null;
  };
  jitter: FeedbackJitter | null;
  jitterGroups?: FeedbackJitterGroup[];
  reviews: {
    commentCount: number;
    ratingCount: number;
    averageScore: number | null;
  };
  asOf: string;
  firstObservedAt?: string | null;
  lastObservedAt?: string | null;
}

export interface FeedbackReview {
  reviewId: string;
  score: number | null;
  comment: string | null;
  publishedDate: string;
  hardwareSnapshot?: {
    gpuName?: string;
    rentedGpuCount?: number;
    country?: string;
    region?: string;
  } | null;
}

export interface FeedbackReviewPage {
  reviews: FeedbackReview[];
  nextCursor: string | null;
}

export interface FeedbackReviewDraft {
  score: number | null;
  comment: string | null;
}
