# Feature Specification: {{FEATURE_NAME}}

**Feature Branch**: `{{FEATURE_NAME}}`
**Created**: {{DATE}}
**Status**: Draft
**Input**: User description: "$ARGUMENTS"

## User Scenarios & Testing *(mandatory)*

<!--
  Prioritize user journeys by business value. Each journey must be independently
  testable: if only one journey ships, it should still provide useful value.
-->

### User Story 1 - [Short Title] (Priority: P1)

[Describe the user journey in plain language]

**Why this priority**: [Explain the value and why it comes first]

**Independent Test**: [Describe how to verify this story independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [observable outcome]
2. **Given** [initial state], **When** [action], **Then** [observable outcome]

---

### User Story 2 - [Short Title] (Priority: P2)

[Describe the user journey in plain language]

**Why this priority**: [Explain the value]

**Independent Test**: [Describe how to verify this story independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [observable outcome]

---

### User Story 3 - [Short Title] (Priority: P3)

[Describe the user journey in plain language]

**Why this priority**: [Explain the value]

**Independent Test**: [Describe how to verify this story independently]

**Acceptance Scenarios**:

1. **Given** [initial state], **When** [action], **Then** [observable outcome]

### Edge Cases

- What happens when [boundary condition]?
- How does the system handle [error condition]?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST [specific capability]
- **FR-002**: System MUST [specific capability]
- **FR-003**: Users MUST be able to [key interaction]
- **FR-004**: System MUST [data requirement]
- **FR-005**: System MUST [observable behavior]

*Example of marking unclear requirements:*

- **FR-006**: System MUST authenticate users via [NEEDS CLARIFICATION: auth method not specified]
- **FR-007**: System MUST retain [NEEDS CLARIFICATION: retention period not specified]

### Key Entities *(include if feature involves data)*

- **[Entity 1]**: [What it represents, key attributes without implementation details]
- **[Entity 2]**: [What it represents, relationships to other entities]

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: [Metric, e.g. "Users can complete primary task in under 2 minutes"]
- **SC-002**: [Metric, e.g. "System supports 1,000 concurrent users"]
- **SC-003**: [Metric, e.g. "95% of users complete the task without support"]
- **SC-004**: [Metric, e.g. "Reduce support tickets about X by 50%"]
