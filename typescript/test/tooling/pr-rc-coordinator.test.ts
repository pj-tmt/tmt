import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { serialize, deserialize } from 'node:v8';
import { describe, expect, it, vi } from 'vite-plus/test';
import * as journal from '../../scripts/pr-rc-journal.mjs';
import {
  coordinatePRRC,
  type RCPublicationInput,
  type RCPublicationPorts,
  type RCPublicationResponse,
  type RCPublicationRecovery,
  type RCUploadResponse,
} from '../../scripts/pr-rc-coordinator.mjs';
import { decodeRCCheckpoint, type RCCheckpointRecovery } from '../../scripts/pr-rc-journal.mjs';
// Independent Python hashlib/json/USTAR/gzip literals; synthetic bytes, no executed binary.
const SOURCE_FILES = {
  'rust/crates/tmt-adapters/src/storage/migrations.rs': 'synthetic migration owner\n',
  'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs': 'synthetic host owner\n',
  'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql': 'synthetic schema\n',
};
const ARCHIVES = [
  'H4sIAAAAAAAC/+3WQU+DMBjGcT4KX6ATRlvPZJJIQtAMYuKxAQxNoJDyTue3t2qiBhO9MA7b87u06aWnf99ST6zqNFPKVq3kTI1j17Ba2RdtrqgnbwGBcy3Ex+rM1yDYiu/9+3kYScE9P/BWcJhIWXe9d5mmV0NtQ7ryd1nqP+kjHWzjwaWgf/pntdXPjWVtY2t7sv6jWf9yywX6X7f/auhHZfRgEAX6/+w/S3dJXiRLzH/J+R/9h7P+eeieBPS/av+drhozYfaj/6/+87hMHxKW5kUZZ9mmr0/Ufyhn/QsRSfS/bv9mIMSP/n/2X96m+xt2H+/LR5bfle43UGzoSIv3/2v+SxEF6H/1/t0PYEISAAAAAAAAAAAAAGfhDRvGnfAAKAAA',
  'H4sIAAAAAAAC/+3WQWuDMBjGcT+KXyCdVmPO0gkNiBtVBjuKOgwzscTXzX37uV02LIxBOw/r87sk5JLTn/clTazqFCtLW7VRyEbzbPpXwzplxonpcehuSJNzFm8mOP88Z8vT87b86/7x7nPfCxzXc1YwDlTa+XvnOg1vhtqGVOXuUuk+qYlG2zhwLehX/bPaqpfGsraxtf2D/oNF/yIUIfpft/+q18fSqN4gCvT/vf9U7pIsT86b/1EY/tC/v5z/Ad+i/3X771TVmAGzH/0v+s/iQj4kTGZ5EafpRtcX79+PFv1HIuLof93+TU+IH/2f9l/s5eGW3ceH4pFld8W8DeQbmuiC/Z/MfyG4QP+r9z9vAAOSAAAAAAAAAAAAAPgX3gHkOVj0ACgAAA==',
  'H4sIAAAAAAAC/+3WQWuDMBjGcT+KXyCdGk13lU6YIG5UGew0RDMa0Cjx7ea+/bLC2HDQk/WwPr9LRARPf55QR6xuFZtuxYsIWTUMrWRNZd6VvqGOnCV41jaKTqc1Pz0viH6ev977nIvAcT1nBceRKmN/71yn8UPTQZKq3V2Wuq9qoqORDlwLOt8/a4x6k4YdpGnM5frns/6jbcTR/7r91303VFr1GlGg/1P/WbpL8iJZZP9FGJ7p35/vv+A++l+3/1bVUo/YfvT/3X8el+lTwtK8KOMs23TNpfr3xXz/A/s5+l+1f90T4kf/v/ov79P9HXuM9+Uzyx9KexsoNjTR8v3/2X8RcIH+V+/f3gBGJAEAAAAAAAAAAADwL3wCHMimGQAoAAA=',
  'H4sIAAAAAAAC/+3WQWuDMBjGcT+KXyBdtDHuKp1QQdyoMthpFHU0zMQSXzf37ed22WZhDNp6WJ/fJSGXnP68L2liZaPYcC0fpWC9eTbtq2GNMv3AdN81V6TJOQ4fhUHweY6mJ+d+8HX/ePeE9DzH5c4M+o62dvzeuUzdm6FdTap0V2niPqmBels7cCnoL/2zyqqX2rJdbSt7jv6Xk/5Dn/vof97+y1bvt0a1BlGg/2/9p8kqzvL4yPkvhfilf2/Sf8AlR//z9t+osjYdZj/6/9l/FhXJfcySLC+iNF3o6vT9e3LSvxThEv3P279pCfGj/4P+i3WyuWF30aZ4YNltMW4D+YIGOmX/B/M/FDJA/7P3P24AHZIAAAAAAAAAAAAA+BfeAVDqJ1sAKAAA',
];
const MANIFEST =
  '{\n  "artifacts": {\n    "tmt-cli-aarch64-apple-darwin.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-aarch64-apple-darwin.tar.gz",\n      "target_triples": [\n        "aarch64-apple-darwin"\n      ],\n      "checksums": {\n        "sha256": "d3b5d904be830b6808e5860c9ade91179d8d00b82740291f6578592bd8223353"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    },\n    "tmt-cli-aarch64-unknown-linux-musl.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-aarch64-unknown-linux-musl.tar.gz",\n      "target_triples": [\n        "aarch64-unknown-linux-musl"\n      ],\n      "checksums": {\n        "sha256": "5839e5ef876128ae8a3daa27573b1f27ece6016a6fbad02be7ee614d2380689f"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    },\n    "tmt-cli-x86_64-apple-darwin.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-x86_64-apple-darwin.tar.gz",\n      "target_triples": [\n        "x86_64-apple-darwin"\n      ],\n      "checksums": {\n        "sha256": "939106922eac7d5d320d62a6a97ad3ecdf3c79529c974ab4bf31eb2363d5ae88"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    },\n    "tmt-cli-x86_64-unknown-linux-musl.tar.gz": {\n      "kind": "executable-zip",\n      "name": "tmt-cli-x86_64-unknown-linux-musl.tar.gz",\n      "target_triples": [\n        "x86_64-unknown-linux-musl"\n      ],\n      "checksums": {\n        "sha256": "a0ea4bafed73aa2039f4222dd61a4ae4f9506b1e0e945002a903b31fc28c1c94"\n      },\n      "assets": [\n        {\n          "path": "tmt"\n        },\n        {\n          "path": "tmt-driver-herdr"\n        },\n        {\n          "path": "LICENSE"\n        },\n        {\n          "path": "NATIVE-INSTALL.md"\n        },\n        {\n          "path": "THIRD-PARTY-NOTICES.txt"\n        }\n      ]\n    }\n  },\n  "releases": [\n    {\n      "app_name": "tmt-cli",\n      "app_version": "5.0.0-alpha.92",\n      "artifacts": [\n        "tmt-cli-aarch64-apple-darwin.tar.gz",\n        "tmt-cli-aarch64-unknown-linux-musl.tar.gz",\n        "tmt-cli-x86_64-apple-darwin.tar.gz",\n        "tmt-cli-x86_64-unknown-linux-musl.tar.gz"\n      ]\n    }\n  ],\n  "tmt_application_schema": {\n    "schema_version": 1,\n    "product": "cli",\n    "source_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",\n    "databases": [\n      {\n        "domain": "tmt-core-db",\n        "version": 48\n      }\n    ],\n    "source_files": [\n      {\n        "path": "rust/crates/tmt-adapters/src/storage/migrations.rs",\n        "sha256": "785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95"\n      },\n      {\n        "path": "rust/crates/tmt-adapters/src/storage/migrations/host_names.rs",\n        "sha256": "409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4"\n      },\n      {\n        "path": "rust/crates/tmt-adapters/src/storage/schema/001_initial.sql",\n        "sha256": "553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513"\n      }\n    ]\n  }\n}\n';
const PREPARATION = {
  repository: 'pj-tmt/tmt',
  run_id: 8001,
  run_attempt: 1,
  api_head_sha: 'ffffffffffffffffffffffffffffffffffffffff',
  source_sha: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  tooling_sha: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
  workflow_id: 702,
  workflow_path: '.github/workflows/native-release-prepare.yml',
  workflow_sha256: '9999999999999999999999999999999999999999999999999999999999999999',
  closure: [
    {
      path: '.github/workflows/native-release-prepare.yml',
      sha256: '9999999999999999999999999999999999999999999999999999999999999999',
    },
    {
      path: 'typescript/pnpm-lock.yaml',
      sha256: '8888888888888888888888888888888888888888888888888888888888888888',
    },
    {
      path: 'typescript/scripts/verify-native-artifact.mjs',
      sha256: '7777777777777777777777777777777777777777777777777777777777777777',
    },
  ],
  run: {
    repository: { full_name: 'pj-tmt/tmt' },
    id: 8001,
    run_attempt: 1,
    head_sha: 'ffffffffffffffffffffffffffffffffffffffff',
    workflow_id: 702,
    path: '.github/workflows/native-release-prepare.yml',
    status: 'completed',
    conclusion: 'success',
  },
  source_snapshot: {
    schema: 1,
    product: 'cli',
    cut: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    version: '5.0.0-alpha.92',
    tag: 'v5.0.0-alpha.92',
    hashes: {
      'rust/crates/tmt-adapters/src/storage/migrations.rs':
        '785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95',
      'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs':
        '409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4',
      'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql':
        '553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513',
    },
  },
  artifacts: [
    {
      id: 801,
      name: 'ordinary-prepared-cli',
      run_id: 8001,
      run_attempt: 1,
      zip_sha256: '6666666666666666666666666666666666666666666666666666666666666666',
      zip_bytes: 10000,
    },
  ],
  targets: [
    {
      target: 'aarch64-apple-darwin',
      host_target: 'aarch64-apple-darwin',
      artifact_id: 801,
      archive_name: 'tmt-cli-aarch64-apple-darwin.tar.gz',
      archive_sha256: 'd3b5d904be830b6808e5860c9ade91179d8d00b82740291f6578592bd8223353',
      archive_bytes: 294,
      manifest_sha256: 'a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5',
      manifest_bytes: 3661,
      binary_sha256: '59994368f1b4fc9cdf6e65f700eda9115916f8f574f3f66e1f95da7fd067eef6',
      output_sha256: 'c7289d7199f5b4f044f924966cda83c1bdf3197e3a2cdeaf6fddb0776688fe96',
      application_schema: {
        schema_version: 1,
        product: 'cli',
        source_sha: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        databases: [{ domain: 'tmt-core-db', version: 48 }],
        source_files: [
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations.rs',
            sha256: '785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs',
            sha256: '409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql',
            sha256: '553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513',
          },
        ],
      },
      notices_sha256: '87c4c0e8b7c53fc56f29b5046e43e5602dee19976fbf4f1a7d5c5345c61ff4e8',
      notices_bytes: 17,
      inventory_sha256: '5555555555555555555555555555555555555555555555555555555555555555',
      verification_reference: 'verified/aarch64-apple-darwin',
    },
    {
      target: 'aarch64-unknown-linux-musl',
      host_target: 'aarch64-unknown-linux-musl',
      artifact_id: 801,
      archive_name: 'tmt-cli-aarch64-unknown-linux-musl.tar.gz',
      archive_sha256: '5839e5ef876128ae8a3daa27573b1f27ece6016a6fbad02be7ee614d2380689f',
      archive_bytes: 298,
      manifest_sha256: 'a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5',
      manifest_bytes: 3661,
      binary_sha256: '59994368f1b4fc9cdf6e65f700eda9115916f8f574f3f66e1f95da7fd067eef6',
      output_sha256: 'c7289d7199f5b4f044f924966cda83c1bdf3197e3a2cdeaf6fddb0776688fe96',
      application_schema: {
        schema_version: 1,
        product: 'cli',
        source_sha: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        databases: [{ domain: 'tmt-core-db', version: 48 }],
        source_files: [
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations.rs',
            sha256: '785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs',
            sha256: '409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql',
            sha256: '553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513',
          },
        ],
      },
      notices_sha256: '87c4c0e8b7c53fc56f29b5046e43e5602dee19976fbf4f1a7d5c5345c61ff4e8',
      notices_bytes: 17,
      inventory_sha256: '5555555555555555555555555555555555555555555555555555555555555555',
      verification_reference: 'verified/aarch64-unknown-linux-musl',
    },
    {
      target: 'x86_64-apple-darwin',
      host_target: 'x86_64-apple-darwin',
      artifact_id: 801,
      archive_name: 'tmt-cli-x86_64-apple-darwin.tar.gz',
      archive_sha256: '939106922eac7d5d320d62a6a97ad3ecdf3c79529c974ab4bf31eb2363d5ae88',
      archive_bytes: 293,
      manifest_sha256: 'a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5',
      manifest_bytes: 3661,
      binary_sha256: '59994368f1b4fc9cdf6e65f700eda9115916f8f574f3f66e1f95da7fd067eef6',
      output_sha256: 'c7289d7199f5b4f044f924966cda83c1bdf3197e3a2cdeaf6fddb0776688fe96',
      application_schema: {
        schema_version: 1,
        product: 'cli',
        source_sha: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        databases: [{ domain: 'tmt-core-db', version: 48 }],
        source_files: [
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations.rs',
            sha256: '785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs',
            sha256: '409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql',
            sha256: '553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513',
          },
        ],
      },
      notices_sha256: '87c4c0e8b7c53fc56f29b5046e43e5602dee19976fbf4f1a7d5c5345c61ff4e8',
      notices_bytes: 17,
      inventory_sha256: '5555555555555555555555555555555555555555555555555555555555555555',
      verification_reference: 'verified/x86_64-apple-darwin',
    },
    {
      target: 'x86_64-unknown-linux-musl',
      host_target: 'x86_64-unknown-linux-musl',
      artifact_id: 801,
      archive_name: 'tmt-cli-x86_64-unknown-linux-musl.tar.gz',
      archive_sha256: 'a0ea4bafed73aa2039f4222dd61a4ae4f9506b1e0e945002a903b31fc28c1c94',
      archive_bytes: 300,
      manifest_sha256: 'a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5',
      manifest_bytes: 3661,
      binary_sha256: '59994368f1b4fc9cdf6e65f700eda9115916f8f574f3f66e1f95da7fd067eef6',
      output_sha256: 'c7289d7199f5b4f044f924966cda83c1bdf3197e3a2cdeaf6fddb0776688fe96',
      application_schema: {
        schema_version: 1,
        product: 'cli',
        source_sha: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        databases: [{ domain: 'tmt-core-db', version: 48 }],
        source_files: [
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations.rs',
            sha256: '785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/migrations/host_names.rs',
            sha256: '409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4',
          },
          {
            path: 'rust/crates/tmt-adapters/src/storage/schema/001_initial.sql',
            sha256: '553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513',
          },
        ],
      },
      notices_sha256: '87c4c0e8b7c53fc56f29b5046e43e5602dee19976fbf4f1a7d5c5345c61ff4e8',
      notices_bytes: 17,
      inventory_sha256: '5555555555555555555555555555555555555555555555555555555555555555',
      verification_reference: 'verified/x86_64-unknown-linux-musl',
    },
  ],
};
const EMPTY =
  '{"predecessor":null,"revision":1,"schema":1,"snapshot":{"approvedProducer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":701,"workflowPath":".github/workflows/pr-rc.yml","workflowSha256":"97cd3946c022af52e42c0237c4768d135f52a0f3b9a39676670ad56f2942a662"},"channels":[{"epoch":{"enabledAtMs":1791291500000,"eventId":6201,"labelId":6101},"pr":234,"reference":"channel/234","sourceRepository":"pj-tmt/tmt","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"enabled"}],"complete":true,"generations":[],"journal":{"checkpointBytes":131072,"checkpoints":3,"terminalEntries":0},"reference":"snapshot/1","repository":"pj-tmt/tmt","unknown":[]},"snapshotDigest":"c489d2d0fd351f60c173f67475987514f0c6ec33dd7d07c6e18cfcd06ac1e5fd","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","terminal":[],"writer":{"attempt":1,"ownerId":77,"producer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":701,"workflowPath":".github/workflows/pr-rc.yml","workflowSha256":"97cd3946c022af52e42c0237c4768d135f52a0f3b9a39676670ad56f2942a662"},"repository":"pj-tmt/tmt","runId":9001}}';
const RESERVED =
  '{"predecessor":{"digest":"162c5a6ab5f481afa66359e94d7ad07939822eb477838fa783d0a37eb6963f15","id":91},"revision":2,"schema":1,"snapshot":{"approvedProducer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":701,"workflowPath":".github/workflows/pr-rc.yml","workflowSha256":"97cd3946c022af52e42c0237c4768d135f52a0f3b9a39676670ad56f2942a662"},"channels":[{"epoch":{"enabledAtMs":1791291500000,"eventId":6201,"labelId":6101},"pr":234,"reference":"channel/234","sourceRepository":"pj-tmt/tmt","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"enabled"}],"complete":true,"generations":[{"disposition":"pending","identity":{"attempt":1,"epoch":{"enabledAtMs":1791291500000,"eventId":6201,"labelId":6101},"pr":234,"producer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":701,"workflowPath":".github/workflows/pr-rc.yml","workflowSha256":"97cd3946c022af52e42c0237c4768d135f52a0f3b9a39676670ad56f2942a662"},"repository":"pj-tmt/tmt","runId":9001,"selection":[{"product":"cli","target":"aarch64-apple-darwin"},{"product":"cli","target":"aarch64-unknown-linux-musl"},{"product":"cli","target":"x86_64-apple-darwin"},{"product":"cli","target":"x86_64-unknown-linux-musl"}],"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"mutation":"none","observations":{"absence":{"artifactIds":[],"generationKey":"a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","reference":"absence/a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","state":"unknown"},"inventory":{"artifactIds":[],"generationKey":"a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","reference":"inventory/a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","state":"unknown"},"reservation":{"artifactIds":[],"generationKey":"a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","reference":"reservation/a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","state":"unknown"},"settlement":{"artifactIds":[],"generationKey":"a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","reference":"settlement/a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c","state":"unknown"}},"resources":[{"artifactId":null,"diagnosticBytes":0,"kind":"payload","manifestBytes":3661,"metadataBytes":4194304,"path":"tmt-pr-rc-payload-v2-pr234-cli-aarch64-apple-darwin-9001-a1","selectionIndex":0,"transportBytes":72351744},{"artifactId":null,"diagnosticBytes":0,"kind":"payload","manifestBytes":3661,"metadataBytes":4194304,"path":"tmt-pr-rc-payload-v2-pr234-cli-aarch64-unknown-linux-musl-9001-a1","selectionIndex":1,"transportBytes":72351744},{"artifactId":null,"diagnosticBytes":0,"kind":"payload","manifestBytes":3661,"metadataBytes":4194304,"path":"tmt-pr-rc-payload-v2-pr234-cli-x86_64-apple-darwin-9001-a1","selectionIndex":2,"transportBytes":72351744},{"artifactId":null,"diagnosticBytes":0,"kind":"payload","manifestBytes":3661,"metadataBytes":4194304,"path":"tmt-pr-rc-payload-v2-pr234-cli-x86_64-unknown-linux-musl-9001-a1","selectionIndex":3,"transportBytes":72351744},{"artifactId":null,"diagnosticBytes":0,"kind":"catalog","manifestBytes":0,"metadataBytes":4194304,"path":"tmt-pr-rc-catalog-v2-pr234","selectionIndex":null,"transportBytes":2097152}],"reuse":[{"artifactId":801,"attempt":1,"repository":"pj-tmt/tmt","runId":8001,"sha256":"6666666666666666666666666666666666666666666666666666666666666666"}]}],"journal":{"checkpointBytes":131072,"checkpoints":3,"terminalEntries":0},"reference":"snapshot/1","repository":"pj-tmt/tmt","unknown":[]},"snapshotDigest":"635b8969ba84e33db3a60768624e1393b3673544f6e0109070b461615e66fab3","sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","terminal":[],"writer":{"attempt":1,"ownerId":77,"producer":{"toolingCommit":"cccccccccccccccccccccccccccccccccccccccc","workflowId":701,"workflowPath":".github/workflows/pr-rc.yml","workflowSha256":"97cd3946c022af52e42c0237c4768d135f52a0f3b9a39676670ad56f2942a662"},"repository":"pj-tmt/tmt","runId":9001}}';
const GOLDEN =
  '{"schema_version":2,"kind":"tmt-pr-rc-catalog","repository":"pj-tmt/tmt","pr":234,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","producer":{"workflow_id":701,"workflow_path":".github/workflows/pr-rc.yml","workflow_sha256":"97cd3946c022af52e42c0237c4768d135f52a0f3b9a39676670ad56f2942a662","tooling_sha":"cccccccccccccccccccccccccccccccccccccccc","run_id":9001,"run_attempt":1},"eligibility":{"label":"rc-build","label_id":6101,"enabled_event_id":6201,"enabled_at_ms":1791291500000},"published_at_ms":1791291600000,"expires_at_ms":1791550800000,"candidates":[{"product":"cli","target":"aarch64-apple-darwin","version":"5.0.0-alpha.92","application_schema":{"schema_version":1,"product":"cli","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","databases":[{"domain":"tmt-core-db","version":48}],"source_files":[{"path":"rust/crates/tmt-adapters/src/storage/migrations.rs","sha256":"785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95"},{"path":"rust/crates/tmt-adapters/src/storage/migrations/host_names.rs","sha256":"409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4"},{"path":"rust/crates/tmt-adapters/src/storage/schema/001_initial.sql","sha256":"553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513"}]},"payload_artifact":{"id":8101,"name":"tmt-pr-rc-payload-v2-pr234-cli-aarch64-apple-darwin-9001-a1","zip_sha256":"1111111111111111111111111111111111111111111111111111111111111111","zip_bytes":9000},"dist_manifest":{"name":"dist-manifest.json","sha256":"a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5","bytes":3661},"archive":{"name":"tmt-cli-aarch64-apple-darwin.tar.gz","sha256":"d3b5d904be830b6808e5860c9ade91179d8d00b82740291f6578592bd8223353","bytes":294},"verification":{"prepare_run_id":8001,"prepare_run_attempt":1,"prepare_tooling_sha":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","complete":true}},{"product":"cli","target":"aarch64-unknown-linux-musl","version":"5.0.0-alpha.92","application_schema":{"schema_version":1,"product":"cli","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","databases":[{"domain":"tmt-core-db","version":48}],"source_files":[{"path":"rust/crates/tmt-adapters/src/storage/migrations.rs","sha256":"785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95"},{"path":"rust/crates/tmt-adapters/src/storage/migrations/host_names.rs","sha256":"409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4"},{"path":"rust/crates/tmt-adapters/src/storage/schema/001_initial.sql","sha256":"553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513"}]},"payload_artifact":{"id":8102,"name":"tmt-pr-rc-payload-v2-pr234-cli-aarch64-unknown-linux-musl-9001-a1","zip_sha256":"2222222222222222222222222222222222222222222222222222222222222222","zip_bytes":9001},"dist_manifest":{"name":"dist-manifest.json","sha256":"a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5","bytes":3661},"archive":{"name":"tmt-cli-aarch64-unknown-linux-musl.tar.gz","sha256":"5839e5ef876128ae8a3daa27573b1f27ece6016a6fbad02be7ee614d2380689f","bytes":298},"verification":{"prepare_run_id":8001,"prepare_run_attempt":1,"prepare_tooling_sha":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","complete":true}},{"product":"cli","target":"x86_64-apple-darwin","version":"5.0.0-alpha.92","application_schema":{"schema_version":1,"product":"cli","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","databases":[{"domain":"tmt-core-db","version":48}],"source_files":[{"path":"rust/crates/tmt-adapters/src/storage/migrations.rs","sha256":"785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95"},{"path":"rust/crates/tmt-adapters/src/storage/migrations/host_names.rs","sha256":"409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4"},{"path":"rust/crates/tmt-adapters/src/storage/schema/001_initial.sql","sha256":"553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513"}]},"payload_artifact":{"id":8103,"name":"tmt-pr-rc-payload-v2-pr234-cli-x86_64-apple-darwin-9001-a1","zip_sha256":"3333333333333333333333333333333333333333333333333333333333333333","zip_bytes":9002},"dist_manifest":{"name":"dist-manifest.json","sha256":"a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5","bytes":3661},"archive":{"name":"tmt-cli-x86_64-apple-darwin.tar.gz","sha256":"939106922eac7d5d320d62a6a97ad3ecdf3c79529c974ab4bf31eb2363d5ae88","bytes":293},"verification":{"prepare_run_id":8001,"prepare_run_attempt":1,"prepare_tooling_sha":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","complete":true}},{"product":"cli","target":"x86_64-unknown-linux-musl","version":"5.0.0-alpha.92","application_schema":{"schema_version":1,"product":"cli","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","databases":[{"domain":"tmt-core-db","version":48}],"source_files":[{"path":"rust/crates/tmt-adapters/src/storage/migrations.rs","sha256":"785c119863d25bf07df836140552f56adf09e1afa137b3f11157dcfe97cb6b95"},{"path":"rust/crates/tmt-adapters/src/storage/migrations/host_names.rs","sha256":"409b432374bf273c3a17455df0fe172f09a9475eb552a852a2a0866a9048dcd4"},{"path":"rust/crates/tmt-adapters/src/storage/schema/001_initial.sql","sha256":"553101feebd4c71b6544d16a606f169e08df5dcf053315d624b00a1f2ac08513"}]},"payload_artifact":{"id":8104,"name":"tmt-pr-rc-payload-v2-pr234-cli-x86_64-unknown-linux-musl-9001-a1","zip_sha256":"4444444444444444444444444444444444444444444444444444444444444444","zip_bytes":9003},"dist_manifest":{"name":"dist-manifest.json","sha256":"a5eb9009782705b87a9c1f0e5a64373a19527456b178386c603f2b972ea073f5","bytes":3661},"archive":{"name":"tmt-cli-x86_64-unknown-linux-musl.tar.gz","sha256":"a0ea4bafed73aa2039f4222dd61a4ae4f9506b1e0e945002a903b31fc28c1c94","bytes":300},"verification":{"prepare_run_id":8001,"prepare_run_attempt":1,"prepare_tooling_sha":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","source_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","complete":true}}]}';
const GOLDEN_HASH = '943b669130183e315e20cbf2e1b9fb484c4983f6e02094a28e9c09d128d5b30f';
const GENERATION_KEY = 'a9106d5d5ced7646dc4dada6eb14607dbc7671a86ed6a9438f1137784708d72c';
const CHARGES = { bytes: 312868864, artifacts: 5 };

const bytes = (s: string) => new TextEncoder().encode(s);
const digest = (b: string | Uint8Array) => createHash('sha256').update(b).digest('hex');
const root = '/repos/pj-tmt/tmt/deployments';
const environment = 'tmt-pr-rc-checkpoint';
const fixedCatalog = JSON.parse(GOLDEN);
const fixedEmpty = JSON.parse(EMPTY);
const targets = fixedCatalog.candidates.map((c: { target: string }) => c.target) as string[];
const writer = fixedEmpty.writer;
const deployment = (id: number, payload: string) => ({
  id,
  repository_url: 'https://api.github.com/repos/pj-tmt/tmt',
  creator: { id: 77 },
  sha: 'a'.repeat(40),
  task: environment,
  environment,
  payload,
  transient_environment: false,
  production_environment: false,
});
const ordinary = {
  id: 7,
  environment: 'production',
  task: 'deploy',
  payload: 'ordinary artifact stays read-only',
};
const inactive = {
  id: 51,
  creator: { id: 77 },
  state: 'inactive',
  environment,
  deployment_url: `${'https://api.github.com'}${root}/91`,
};
const checkpointSteps = [
  ['GET', `${root}?per_page=100&page=1`, 200, [ordinary, deployment(91, EMPTY)]],
  ['GET', `${root}/91`, 200, deployment(91, EMPTY)],
  ['POST', root, 201, deployment(42, RESERVED)],
  ['GET', `${root}/42`, 200, deployment(42, RESERVED)],
  [
    'GET',
    `${root}?per_page=100&page=1`,
    200,
    [ordinary, deployment(91, EMPTY), deployment(42, RESERVED)],
  ],
  ['POST', `${root}/91/statuses`, 201, inactive],
  ['GET', `${root}/91/statuses?per_page=100&page=1`, 200, [inactive]],
  [
    'GET',
    `${root}?per_page=100&page=1`,
    200,
    [ordinary, deployment(91, EMPTY), deployment(42, RESERVED)],
  ],
  ['GET', `${root}/91`, 200, deployment(91, EMPTY)],
  ['DELETE', `${root}/91`, 204, null],
  ['GET', `${root}/91`, 404, { message: 'Not Found' }],
  ['GET', `${root}?per_page=100&page=1`, 200, [ordinary, deployment(42, RESERVED)]],
] as const;

function eligible() {
  return {
    workflow: {
      path: '.github/workflows/pr-rc.yml',
      tooling_sha: 'c'.repeat(40),
      body: 'synthetic publisher workflow\n',
    },
    pull: {
      number: 234,
      state: 'open',
      base: { repo: { full_name: 'pj-tmt/tmt' } },
      head: { repo: { full_name: 'pj-tmt/tmt' }, sha: 'a'.repeat(40) },
      labels: [{ id: 6101, name: 'rc-build' }],
    },
    timeline: [
      {
        id: 6201,
        event: 'labeled',
        label: { id: 6101, name: 'rc-build' },
        created_at: '2026-10-06T12:58:20Z',
      },
    ],
    run: {
      repository: { full_name: 'pj-tmt/tmt' },
      id: 9001,
      run_attempt: 1,
      workflow_id: 701,
      path: '.github/workflows/pr-rc.yml',
      head_sha: 'c'.repeat(40),
      head_branch: 'main',
      event: 'workflow_dispatch',
      status: 'in_progress',
    },
  };
}

function fixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-1947-source-'));
  const sourceRoot = path.join(directory, 'source');
  for (const [name, text] of Object.entries(SOURCE_FILES)) {
    fs.mkdirSync(path.dirname(path.join(sourceRoot, name)), { recursive: true });
    fs.writeFileSync(path.join(sourceRoot, name), text);
  }
  const manifestPath = path.join(directory, 'dist-manifest.json');
  fs.writeFileSync(manifestPath, MANIFEST);
  const archives = targets.map((target, i) => {
    const file = path.join(directory, `tmt-cli-${target}.tar.gz`);
    fs.writeFileSync(file, Buffer.from(ARCHIVES[i], 'base64'));
    return { target, path: file };
  });
  const input: RCPublicationInput = {
    identity: JSON.parse(RESERVED).snapshot.generations[0].identity,
    writer,
    snapshot: JSON.parse(EMPTY).snapshot,
    recorded: [
      {
        id: 91,
        bytes: bytes(EMPTY),
        digest: digest(EMPTY),
        charges: { bytes: 393216, artifacts: 0 },
      },
    ],
    terminal: [],
    sourceRoot,
    manifestPath,
    archives,
    startedAtMs: 100,
    publishedAtMs: 1791291600000,
    expiresAtMs: 1791550800000,
  };
  const approval = {
    reference: 'review/immutable-synthetic',
    producer: writer.producer,
    preparation: Object.fromEntries(
      ['workflow_id', 'workflow_path', 'workflow_sha256', 'tooling_sha', 'closure'].map((key) => [
        key,
        PREPARATION[key as keyof typeof PREPARATION],
      ])
    ),
  } as Awaited<ReturnType<RCPublicationPorts['approval']['capture']>>;
  const preparation = structuredClone(PREPARATION);
  let eligibilityValue = eligible();
  let checkpointIndex = 0;
  let eligibilityCount = 0;
  let uploadCount = 0;
  let tick = 100;
  const events: string[] = [];
  const retained: (RCCheckpointRecovery | RCPublicationRecovery)[] = [];
  const uploaded: Parameters<RCPublicationPorts['upload']>[0][] = [];
  const artifactRows = new Map<number, Record<string, unknown>>();
  const recoveryFile = path.join(directory, 'checkpoint-recovery.bin');
  const load = (): RCCheckpointRecovery | RCPublicationRecovery | null =>
    fs.existsSync(recoveryFile) ? deserialize(fs.readFileSync(recoveryFile)) : null;
  let saveFault: ((s: RCCheckpointRecovery | RCPublicationRecovery) => boolean) | null = null;
  let beforeObserve: ((kind: string, id: number | null) => Promise<void>) | null = null;
  let afterUpload: ((r: RCUploadResponse, n: number) => void) | null = null;
  let failUpload = 0;
  let late: ((v: ReturnType<typeof eligible>, n: number) => void) | null = null;
  let observeFault: ((r: RCPublicationResponse, kind: string) => void) | null = null;
  let lostResponse: Promise<RCUploadResponse> | null = null;
  const response = (value: unknown): RCPublicationResponse => ({
    status: 200,
    body: bytes(JSON.stringify(value)),
    elapsedMs: 1,
    nextPage: null,
    reference: 'raw/synthetic',
    observedAtMs: 1791291600001,
    authenticated: { repository: 'pj-tmt/tmt', ownerId: 77 },
  });
  const ports: RCPublicationPorts = {
    checkpoint: {
      now: () => tick,
      custody: {
        capture: async () => ({
          writer,
          reference: 'custody/synthetic',
          concurrencyDomain: 'tmt-pr-rc-writers',
          cancelInProgress: false,
        }),
      },
      recovery: {
        load: async () => load(),
        save: async (state) => {
          const publication = 'publication' in state ? state.publication : null;
          events.push(
            publication
              ? `save:${publication.phase}:${publication.uploads.at(-1)?.phase ?? 'none'}`
              : `checkpoint-save:${state.phase}`
          );
          if (saveFault?.(state)) throw new Error('Injected durable write failure');
          const temporary = `${recoveryFile}.tmp`;
          const fd = fs.openSync(temporary, 'wx', 0o600);
          try {
            fs.writeFileSync(fd, serialize(state));
            fs.fsyncSync(fd);
          } finally {
            fs.closeSync(fd);
          }
          fs.renameSync(temporary, recoveryFile);
          const dir = fs.openSync(directory, 'r');
          try {
            fs.fsyncSync(dir);
          } finally {
            fs.closeSync(dir);
          }
          retained.push(structuredClone(state));
        },
      },
      transport: async (request) => {
        const step = checkpointSteps[checkpointIndex++];
        expect(step, 'Unexpected checkpoint effect').toBeDefined();
        expect([request.method, request.path]).toEqual(step.slice(0, 2));
        if (request.method === 'POST' && request.path === root) {
          const body = JSON.parse(new TextDecoder().decode(request.body!));
          expect(body).toEqual({
            ref: 'a'.repeat(40),
            task: environment,
            auto_merge: false,
            required_contexts: [],
            payload: RESERVED,
            environment,
            description: 'Unarmed PR RC checkpoint candidate',
            transient_environment: false,
            production_environment: false,
          });
          expect(decodeRCCheckpoint(bytes(body.payload)).snapshot.generations[0].resources).toEqual(
            JSON.parse(RESERVED).snapshot.generations[0].resources
          );
        }
        if (request.method === 'POST' && request.path.endsWith('/statuses'))
          expect(JSON.parse(new TextDecoder().decode(request.body!))).toEqual({
            state: 'inactive',
            auto_inactive: false,
            environment,
          });
        expect(request.timeoutMs).toBeLessThanOrEqual(10000);
        events.push(`checkpoint:${request.method}:${request.path}`);
        return {
          status: step[2],
          body: bytes(JSON.stringify(step[3])),
          elapsedMs: 1,
          nextPage: null,
          authenticated: { repository: 'pj-tmt/tmt', ownerId: 77 },
        };
      },
    },
    approval: {
      capture: async () => {
        events.push('approval');
        return structuredClone(approval);
      },
    },
    observe: async (request) => {
      events.push(`observe:${request.kind}:${request.artifactId ?? 'none'}`);
      await beforeObserve?.(request.kind, request.artifactId);
      let value: unknown;
      if (request.kind === 'eligibility') {
        eligibilityCount++;
        const v = structuredClone(eligibilityValue);
        late?.(v, eligibilityCount);
        value = v;
      } else if (request.kind === 'preparation') value = preparation;
      else if (request.kind === 'artifact') value = artifactRows.get(request.artifactId!);
      else {
        const row = artifactRows.get(request.artifactId!)!;
        value = {
          repository: 'pj-tmt/tmt',
          run_id: 9001,
          run_attempt: 1,
          artifact_id: request.artifactId,
          zip_sha256: row.zip_sha256,
          zip_bytes: row.zip_bytes,
          settlement_reference: `ended/${request.artifactId}`,
          writer_ended_at_ms: 1791291600000,
          inventory_reference: `inventory/${request.artifactId}`,
        };
      }
      const r = response(value);
      observeFault?.(r, request.kind);
      return r;
    },
    upload: async (request) => {
      const n = ++uploadCount;
      const state = load() as RCPublicationRecovery;
      expect(state.publication.uploads.at(-1)?.phase).toBe('intent');
      expect(state.publication.unknown).toBe(true);
      expect(digest(state.candidate.bytes)).toBe(digest(RESERVED));
      expect(state.publication.reservationId).toBe(42);
      expect(state.candidate.charges).toEqual(CHARGES);
      events.push(`upload:${request.name}`);
      uploaded.push(structuredClone(request));
      if (failUpload === n) throw new Error('Injected lost upload response');
      if (lostResponse) return lostResponse;
      const members = request.files.map((file) => ({
        name: file.name,
        sha256: digest(file.bytes),
        bytes: file.bytes.length,
      }));
      const id = 8100 + n;
      const value = {
        id,
        repository: 'pj-tmt/tmt',
        run_id: 9001,
        run_attempt: 1,
        name: request.name,
        zip_sha256: String(n).repeat(64),
        zip_bytes: 8999 + n,
        members,
      };
      const r = { ...response(value), status: 201, id };
      afterUpload?.(r, n);
      artifactRows.set(id, value);
      return r;
    },
  };
  return {
    input,
    ports,
    approval,
    preparation,
    events,
    retained,
    uploaded,
    directory,
    load,
    checkpointCount: () => checkpointIndex,
    setEligibility: (v: ReturnType<typeof eligible>) => {
      eligibilityValue = v;
    },
    saveFault: (f: typeof saveFault) => {
      saveFault = f;
    },
    beforeObserve: (f: typeof beforeObserve) => {
      beforeObserve = f;
    },
    afterUpload: (f: typeof afterUpload) => {
      afterUpload = f;
    },
    failUpload: (n: number) => {
      failUpload = n;
    },
    late: (f: typeof late) => {
      late = f;
    },
    observeFault: (f: typeof observeFault) => {
      observeFault = f;
    },
    lostResponse: (p: Promise<RCUploadResponse>) => {
      lostResponse = p;
    },
    clock: (n: number) => {
      tick = n;
    },
    dispose: () => fs.rmSync(directory, { recursive: true }),
  };
}

async function withFixture(run: (f: ReturnType<typeof fixture>) => Promise<void>) {
  const f = fixture();
  try {
    await run(f);
  } finally {
    f.dispose();
  }
}

function publication(result: Awaited<ReturnType<typeof coordinatePRRC>>) {
  expect(result.recovery && 'publication' in result.recovery).toBe(true);
  return (result.recovery as RCPublicationRecovery).publication;
}

describe('trusted reuse-only source coordinator', () => {
  it('independent literal catalog, descriptors, complete reservation and exact catalog-last order', async () => {
    await withFixture(async (f) => {
      f.beforeObserve(async (kind, id) => {
        if (kind === 'artifact') {
          const state = f.load() as RCPublicationRecovery;
          expect(state.publication.uploads.at(-1)?.id).toBe(id);
          expect(state.publication.uploads.at(-1)?.phase).toBe('returned');
          expect(state.publication.uploads.at(-1)?.response?.body).toBeInstanceOf(Uint8Array);
        }
      });
      const result = await coordinatePRRC(f.input, f.ports);
      expect(result.reason).toBeUndefined();
      expect(result.status).toBe('readback-confirmed');
      expect(result.mode).toBe('unarmed');
      expect(result.charges).toEqual(CHARGES);
      const p = publication(result);
      expect(p.generationKey).toBe(GENERATION_KEY);
      expect(p.phase).toBe('readback-confirmed');
      expect(p.unknown).toBe(false);
      expect(p.uploads.map((u) => [u.kind, u.id, u.phase])).toEqual([
        ['payload', 8101, 'finalized'],
        ['payload', 8102, 'finalized'],
        ['payload', 8103, 'finalized'],
        ['payload', 8104, 'finalized'],
        ['catalog', 8105, 'finalized'],
      ]);
      const actual = f.uploaded.at(-1)!.files[0];
      expect(actual.name).toBe('catalog.json');
      expect(new TextDecoder().decode(actual.bytes)).toBe(GOLDEN);
      expect(digest(actual.bytes)).toBe(GOLDEN_HASH);
      expect(
        f.uploaded
          .slice(0, 4)
          .map((u) =>
            u.files.map((a) => ({ name: a.name, sha256: digest(a.bytes), bytes: a.bytes.length }))
          )
      ).toEqual(
        fixedCatalog.candidates.map((c: { dist_manifest: unknown; archive: unknown }) => [
          c.dist_manifest,
          c.archive,
        ])
      );
      expect(f.checkpointCount()).toBe(12);
      expect(f.events.indexOf('checkpoint-save:complete')).toBeLessThan(
        f.events.findIndex((e) => e.startsWith('upload:'))
      );
      expect(f.events.filter((e) => e.startsWith('upload:'))).toEqual([
        ...fixedCatalog.candidates.map(
          (c: { payload_artifact: { name: string } }) => `upload:${c.payload_artifact.name}`
        ),
        'upload:tmt-pr-rc-catalog-v2-pr234',
      ]);
      for (let i = 0; i < 5; i++) {
        const upload = f.events.indexOf(`upload:${f.uploaded[i].name}`);
        expect(f.events[upload - 1]).toBe('save:uploading:intent');
        expect(f.events[upload + 1]).toBe('save:uploading:returned');
        expect(f.events[upload + 2]).toBe(`observe:artifact:${8101 + i}`);
        const end = f.events.indexOf(`observe:finalization:${8101 + i}`);
        if (i < 4) expect(end).toBeLessThan(f.events.indexOf(`upload:${f.uploaded[i + 1].name}`));
      }
      expect(f.load()).toEqual(result.recovery);
      expect(fs.readFileSync(f.input.manifestPath, 'utf8')).toBe(MANIFEST);
      expect(fs.existsSync(f.input.archives[0].path)).toBe(true);
      const replay = await coordinatePRRC(f.input, f.ports);
      expect(replay.status).toBe('frozen');
      expect(replay.charges).toEqual(CHARGES);
      expect(f.uploaded).toHaveLength(5);
    });
  });

  it.each(['commit throws', 'journal recovery load fails'] as const)(
    'retained recovery keeps original charges when %s',
    async (failure) => {
      await withFixture(async (f) => {
        const first = await coordinatePRRC(f.input, f.ports);
        expect(first.status).toBe('readback-confirmed');
        const retained = f.load();
        const events = [...f.events];
        const commit =
          failure === 'commit throws'
            ? vi
                .spyOn(journal, 'commitRCCheckpoint')
                .mockRejectedValueOnce(new Error('Injected checkpoint commit failure'))
            : null;
        if (failure === 'journal recovery load fails') {
          let loads = 0;
          f.ports.checkpoint.recovery.load = async () => {
            if (++loads === 2) throw new Error('Injected journal recovery load failure');
            return retained;
          };
        }
        try {
          const result = await coordinatePRRC(f.input, f.ports);
          expect(result.status).toBe('frozen');
          expect(result.charges).toEqual(CHARGES);
          expect(result.recovery).toEqual(retained);
          expect(f.load()).toEqual(retained);
          expect(f.events).toEqual(events);
          expect(f.uploaded).toHaveLength(5);
        } finally {
          commit?.mockRestore();
        }
      });
    }
  );

  const eligibilityNegatives: [string, (v: ReturnType<typeof eligible>) => void][] = [
    [
      'wrong workflow blob',
      (v) => {
        v.workflow.body += 'PR modification';
      },
    ],
    [
      'stale head',
      (v) => {
        v.pull.head.sha = 'd'.repeat(40);
      },
    ],
    [
      'fork',
      (v) => {
        v.pull.head.repo.full_name = 'fork/tmt';
      },
    ],
    [
      'closed',
      (v) => {
        v.pull.state = 'closed';
      },
    ],
    [
      'disabled',
      (v) => {
        v.pull.labels = [];
      },
    ],
    [
      'recreated label',
      (v) => {
        v.pull.labels[0].id++;
      },
    ],
    [
      'remove/readd',
      (v) => {
        v.timeline.push(
          { ...v.timeline[0], id: 6202, event: 'unlabeled', created_at: '2026-10-06T13:39:20Z' },
          { ...v.timeline[0], id: 6203, created_at: '2026-10-06T13:40:20Z' }
        );
      },
    ],
    [
      'ambiguous same second',
      (v) => {
        v.timeline.push({ ...v.timeline[0], id: 6202 });
      },
    ],
    [
      'wrong current attempt',
      (v) => {
        v.run.run_attempt++;
      },
    ],
    [
      'unknown tooling',
      (v) => {
        v.run.head_sha = 'd'.repeat(40);
      },
    ],
    [
      'non-main producer',
      (v) => {
        v.run.head_branch = 'feature';
      },
    ],
    [
      'missing enable event',
      (v) => {
        v.timeline = [];
      },
    ],
    [
      'oversized timeline',
      (v) => {
        v.timeline = Array.from({ length: 101 }, () => v.timeline[0]);
      },
    ],
    [
      'invalid calendar date',
      (v) => {
        v.timeline[0].created_at = '2026-02-30T12:58:20Z';
      },
    ],
  ];
  it.each(eligibilityNegatives)(
    '%s refuses before any checkpoint/write/upload',
    async (_name, mutate) => {
      await withFixture(async (f) => {
        const v = eligible();
        mutate(v);
        f.setEligibility(v);
        const r = await coordinatePRRC(f.input, f.ports);
        expect(r.status).toBe('refused');
        expect(f.checkpointCount()).toBe(0);
        expect(f.retained).toHaveLength(0);
        expect(f.uploaded).toHaveLength(0);
      });
    }
  );

  it('accepts a complete authenticated one-page timeline at the frozen 100-event bound', async () => {
    await withFixture(async (f) => {
      const v = eligible();
      v.timeline.push(
        ...Array.from({ length: 99 }, (_, i) => ({
          ...v.timeline[0],
          id: 6300 + i,
          event: 'commented',
        }))
      );
      f.setEligibility(v);
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('readback-confirmed');
      expect(f.uploaded).toHaveLength(5);
    });
  });

  const preparationNegatives: [string, (f: ReturnType<typeof fixture>) => void][] = [
    [
      'unknown external approval',
      (f) => {
        f.approval.producer = { ...writer.producer, workflowSha256: 'd'.repeat(64) };
      },
    ],
    [
      'PR tooling closure',
      (f) => {
        f.preparation.closure[1].sha256 = 'd'.repeat(64);
      },
    ],
    [
      'unknown upstream tooling',
      (f) => {
        f.preparation.tooling_sha = 'd'.repeat(40);
      },
    ],
    [
      'wrong preparation attempt',
      (f) => {
        f.preparation.run.run_attempt = 2;
      },
    ],
    [
      'failed preparation run',
      (f) => {
        f.preparation.run.conclusion = 'failure';
      },
    ],
    [
      'synthetic head substituted for source',
      (f) => {
        f.preparation.source_sha = f.preparation.api_head_sha;
      },
    ],
    [
      'missing target',
      (f) => {
        f.preparation.targets.pop();
      },
    ],
    [
      'duplicate target',
      (f) => {
        f.preparation.targets[1] = f.preparation.targets[0];
      },
    ],
    [
      'wrong host',
      (f) => {
        f.preparation.targets[0].host_target = targets[2];
      },
    ],
    [
      'missing final notices proof',
      (f) => {
        f.preparation.targets[0].notices_sha256 = '';
      },
    ],
    [
      'unknown binary proof',
      (f) => {
        f.preparation.targets[0].binary_sha256 = '';
      },
    ],
    [
      'schema mismatch',
      (f) => {
        f.preparation.targets[0].application_schema = {
          ...f.preparation.targets[0].application_schema,
          product: 'remote',
        };
      },
    ],
    [
      'raw archive hash mismatch',
      (f) => {
        f.preparation.targets[0].archive_sha256 = 'd'.repeat(64);
      },
    ],
    [
      'raw archive length mismatch',
      (f) => {
        f.preparation.targets[0].archive_bytes++;
      },
    ],
    [
      'manifest hash mismatch',
      (f) => {
        f.preparation.targets[0].manifest_sha256 = 'd'.repeat(64);
      },
    ],
    [
      'ordinary artifact wrong attempt',
      (f) => {
        f.preparation.artifacts[0].run_attempt++;
      },
    ],
    [
      'missing source closure input',
      (f) => {
        delete (f.preparation.source_snapshot.hashes as Record<string, string>)[
          'rust/crates/tmt-adapters/src/storage/migrations.rs'
        ];
      },
    ],
    [
      'version mismatch',
      (f) => {
        f.preparation.source_snapshot.version = '5.0.0-alpha.93';
      },
    ],
    [
      'manifest unknown bytes',
      (f) => {
        fs.appendFileSync(f.input.manifestPath, ' ');
      },
    ],
    [
      'archive changed after final verification',
      (f) => {
        fs.appendFileSync(f.input.archives[0].path, 'changed');
      },
    ],
    [
      'linked archive input',
      (f) => {
        const file = f.input.archives[0].path;
        fs.renameSync(file, `${file}.original`);
        fs.symlinkSync(`${file}.original`, file);
      },
    ],
    [
      'out of order input',
      (f) => {
        f.input.archives.reverse();
      },
    ],
    [
      'extra target selection',
      (f) => {
        f.input.identity.selection.push({ product: 'remote', target: targets[0] });
      },
    ],
    [
      'unresolved original resource charge',
      (f) => {
        f.input.snapshot.unknown.push({ reference: 'unknown/original', bytes: 500, artifacts: 1 });
      },
    ],
  ];
  it.each(preparationNegatives)('%s refuses with no publication effects', async (_name, mutate) => {
    await withFixture(async (f) => {
      mutate(f);
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('refused');
      expect(f.checkpointCount()).toBe(0);
      expect(f.uploaded).toHaveLength(0);
      expect(fs.existsSync(f.input.manifestPath)).toBe(true);
    });
  });

  it.each([
    'partial',
    'denied',
    'unauthenticated',
    'duplicates',
    'missing export',
    'self-declared complete',
    'oversized',
  ] as const)('%s observation refuses precisely', async (kind) => {
    await withFixture(async (f) => {
      f.observeFault((r, phase) => {
        if (phase !== 'preparation') return;
        if (kind === 'partial') r.nextPage = 2;
        if (kind === 'denied') r.status = 403;
        if (kind === 'unauthenticated') r.authenticated.ownerId++;
        if (kind === 'duplicates') r.body = bytes('{"source_sha":"a","source_sha":"b"}');
        if (kind === 'missing export') r.body = bytes('null');
        if (kind === 'self-declared complete') r.body = bytes('{"complete":true}');
        if (kind === 'oversized') r.body = new Uint8Array(1024 * 1024 + 1);
      });
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('refused');
      expect(f.uploaded).toHaveLength(0);
      if (kind === 'missing export') expect(r.reason).toContain('native-release-prepare.yml');
      expect(r.observations?.at(-1)?.response.body.length).toBeGreaterThan(0);
    });
  });

  it.each([
    'ordinary-id',
    'wrong-attempt',
    'wrong-run',
    'wrong-name',
    'oversized-zip',
    'extra-member',
    'wrong-member-size',
    'wrong-member-hash',
    'missing-returned-id',
    'unauthenticated',
    'readback-mismatch',
    'unknown-settlement',
  ] as const)('%s freezes actual returned response/ID and unchanged charges', async (kind) => {
    await withFixture(async (f) => {
      f.afterUpload((r) => {
        const value = JSON.parse(new TextDecoder().decode(r.body));
        if (kind === 'ordinary-id') {
          r.id = 801;
          value.id = 801;
        }
        if (kind === 'wrong-attempt') value.run_attempt++;
        if (kind === 'wrong-run') value.run_id++;
        if (kind === 'wrong-name') value.name = 'ordinary-prepared-cli';
        if (kind === 'oversized-zip') value.zip_bytes = 69 * 1024 * 1024 + 1;
        if (kind === 'extra-member')
          value.members.push({ name: 'extra', sha256: 'a'.repeat(64), bytes: 1 });
        if (kind === 'wrong-member-size') value.members[0].bytes++;
        if (kind === 'wrong-member-hash') value.members[1].sha256 = 'a'.repeat(64);
        if (kind === 'missing-returned-id') r.id = null;
        if (kind === 'unauthenticated') r.authenticated.ownerId++;
        r.body = bytes(JSON.stringify(value));
      });
      f.observeFault((r, phase) => {
        if (kind === 'readback-mismatch' && phase === 'artifact') {
          const v = JSON.parse(new TextDecoder().decode(r.body));
          v.zip_sha256 = 'd'.repeat(64);
          r.body = bytes(JSON.stringify(v));
        }
        if (kind === 'unknown-settlement' && phase === 'finalization')
          r.body = bytes('{"custody":true,"complete":true,"status":202}');
      });
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('frozen');
      expect(r.charges).toEqual(CHARGES);
      expect(f.uploaded).toHaveLength(1);
      const p = publication(r);
      if (kind === 'missing-returned-id') expect(p.uploads[0].rawReturnedId).toBe(8101);
      expect(p.unknown).toBe(true);
      expect(p.phase).toBe('frozen');
      expect(p.uploads[0].response?.body).toBeInstanceOf(Uint8Array);
      expect(f.load()).toEqual(r.recovery);
      expect(fs.existsSync(f.input.archives[0].path)).toBe(true);
      expect(f.events.filter((s) => s.includes('DELETE')).every((s) => s.endsWith('/91'))).toBe(
        true
      );
      const replay = await coordinatePRRC(f.input, f.ports);
      expect(replay.status).toBe('frozen');
      expect(f.uploaded).toHaveLength(1);
    });
  });

  it.each([1, 2, 4, 5])(
    'lost response at upload %i preserves every original ID and unknown intent',
    async (n) => {
      await withFixture(async (f) => {
        f.failUpload(n);
        const r = await coordinatePRRC(f.input, f.ports);
        expect(r.status).toBe('frozen');
        expect(r.charges).toEqual(CHARGES);
        expect(f.uploaded).toHaveLength(n);
        const p = publication(r);
        expect(p.uploads.slice(0, -1).map((u) => u.id)).toEqual(
          Array.from({ length: n - 1 }, (_, i) => 8101 + i)
        );
        expect(p.uploads.at(-1)?.id).toBeNull();
        expect(p.uploads.at(-1)?.phase).toBe('intent');
        expect(p.unknown).toBe(true);
        expect(f.load()).toEqual(r.recovery);
        const replay = await coordinatePRRC(f.input, f.ports);
        expect(replay.status).toBe('frozen');
        expect(f.uploaded).toHaveLength(n);
      });
    }
  );

  it.each(['intent', 'returned', 'readback', 'finalized'] as const)(
    'durable %s crash window never permits next asynchronous effect',
    async (phase) => {
      await withFixture(async (f) => {
        f.saveFault((s) => 'publication' in s && s.publication.uploads.at(-1)?.phase === phase);
        const r = await coordinatePRRC(f.input, f.ports);
        expect(r.status).toBe('frozen');
        expect(r.reason).toContain('Durable recovery unconfirmed');
        expect(r.charges).toEqual(CHARGES);
        expect(f.uploaded).toHaveLength(phase === 'intent' ? 0 : 1);
        const p = publication(r);
        expect(p.uploads.at(-1)?.phase).toBe(phase);
        if (phase === 'returned') {
          expect(p.uploads[0].id).toBe(8101);
          expect(p.uploads[0].response).not.toBeNull();
          expect(f.events.some((e) => e === 'observe:artifact:8101')).toBe(false);
        }
        if (phase === 'readback')
          expect(f.events.some((e) => e === 'observe:finalization:8101')).toBe(false);
        expect(f.load()).not.toEqual(r.recovery);
        expect((f.load() as RCPublicationRecovery).candidate.charges).toEqual(CHARGES);
      });
    }
  );

  it.each(['close', 'disable', 'advance', 'readd'] as const)(
    'late %s freezes before catalog and preserves four payloads',
    async (kind) => {
      await withFixture(async (f) => {
        f.late((v, n) => {
          if (n !== 6) return;
          if (kind === 'close') v.pull.state = 'closed';
          if (kind === 'disable') v.pull.labels = [];
          if (kind === 'advance') v.pull.head.sha = 'd'.repeat(40);
          if (kind === 'readd')
            v.timeline.push({ ...v.timeline[0], id: 6202, created_at: '2026-10-06T13:40:20Z' });
        });
        const r = await coordinatePRRC(f.input, f.ports);
        expect(r.status).toBe('frozen');
        expect(f.uploaded).toHaveLength(4);
        expect(r.charges).toEqual(CHARGES);
        expect(publication(r).uploads.map((u) => u.id)).toEqual([8101, 8102, 8103, 8104]);
        expect(publication(r).catalog).toBeNull();
      });
    }
  );

  it('asynchronous observer crash after returned ID reads original durable bytes before rejection', async () => {
    await withFixture(async (f) => {
      let release!: () => void;
      let entered!: () => void;
      const arrival = new Promise<void>((r) => {
        entered = r;
      });
      const gate = new Promise<void>((r) => {
        release = r;
      });
      f.beforeObserve(async (kind) => {
        if (kind !== 'artifact') return;
        entered();
        await gate;
        throw new Error('Injected readback crash');
      });
      const running = coordinatePRRC(f.input, f.ports);
      await Promise.race([
        arrival,
        running.then((r) => {
          throw new Error(`Did not reach async barrier: ${r.reason}`);
        }),
      ]);
      const state = f.load() as RCPublicationRecovery;
      expect(state.publication.uploads[0].id).toBe(8101);
      expect(state.publication.uploads[0].response?.body).toEqual(
        bytes(
          JSON.stringify({
            id: 8101,
            repository: 'pj-tmt/tmt',
            run_id: 9001,
            run_attempt: 1,
            name: fixedCatalog.candidates[0].payload_artifact.name,
            zip_sha256: '1'.repeat(64),
            zip_bytes: 9000,
            members: [fixedCatalog.candidates[0].dist_manifest, fixedCatalog.candidates[0].archive],
          })
        )
      );
      expect(f.uploaded).toHaveLength(1);
      release();
      const r = await running;
      expect(r.status).toBe('frozen');
      expect(r.charges).toEqual(CHARGES);
      expect(publication(r).uploads[0].id).toBe(8101);
      expect(f.uploaded).toHaveLength(1);
    });
  });

  it('close during catalog upload/readback freezes catalog ownership and forbids replay', async () => {
    await withFixture(async (f) => {
      f.late((v, n) => {
        if (n === 7) v.pull.state = 'closed';
      });
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('frozen');
      expect(r.charges).toEqual(CHARGES);
      expect(f.uploaded).toHaveLength(5);
      expect(publication(r).uploads.at(-1)?.id).toBe(8105);
      expect(publication(r).catalog).toEqual(fixedCatalog);
      const replay = await coordinatePRRC(f.input, f.ports);
      expect(replay.status).toBe('frozen');
      expect(f.uploaded).toHaveLength(5);
    });
  });

  it('expiry, deadline and partial reservation never permit catalog upload', async () => {
    await withFixture(async (f) => {
      f.input.expiresAtMs = f.input.publishedAtMs + 1;
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('frozen');
      expect(f.uploaded).toHaveLength(4);
      expect(r.charges).toEqual(CHARGES);
    });
    await withFixture(async (f) => {
      f.clock(600100);
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('refused');
      expect(f.uploaded).toHaveLength(0);
    });
    await withFixture(async (f) => {
      f.ports.checkpoint.transport = async () => ({
        status: 403,
        body: bytes('denied'),
        elapsedMs: 1,
        nextPage: null,
        authenticated: { repository: 'pj-tmt/tmt', ownerId: 77 },
      });
      const r = await coordinatePRRC(f.input, f.ports);
      expect(r.status).toBe('refused');
      expect(r.charges).toEqual(CHARGES);
      expect(f.uploaded).toHaveLength(0);
    });
  });
});
