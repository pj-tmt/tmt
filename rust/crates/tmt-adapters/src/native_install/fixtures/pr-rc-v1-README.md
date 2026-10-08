# PR RC revision-1 fixture

`pr-rc-catalog-v1.json` is copied byte for byte from Infra's frozen
[#1889 wire review packet](https://github.com/pj-tmt/tmt/issues/1889#issuecomment-6016822246).
Its SHA-256 is `1953dc982c3ad971ee2006e8bd2549b4a656839c6a2caa4d274e822e91fa4bae`.
All repository, workflow, run and artifact identities are synthetic. It supplies
metadata parsing controls, not live producer approval or native archive evidence.
The tests inject the synthetic approval explicitly; production never registers it.

`pr-rc-catalog-v2.json` and `pr-rc-api-v2.json` are byte-exact revision-2
synthetic fixtures supplied for the accepted eligibility correction. Their
SHA-256 values are `3e1683828ba6c49dba7ecf40f6da49f52e612e8be6192bc3383e7de8cec6f71e`
and `ea61b00baae3745efceb0d32c0e30bb6f16d46805ae467b2c7a9bdea3a630fb3`.
The [revision-2 handoff](https://github.com/pj-tmt/tmt/issues/1889#issuecomment-6017493917)
records the frozen packet. Revision 1 is a negative control: live acquisition
requires version 2 and its actual current enable epoch. Native resolver tests
construct separate ZIP and native archive fixtures; these metadata-only examples
do not assert real build, API, archive or installation success.
