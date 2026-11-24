<!-- grovedb-structure-preview -->
🌳 **GroveDB structure**

This pull request changes the described GroveDB structure. **[Open it in the structure viewer](https://dashpay.github.io/grovedb-structure-viewer/?base=c47db15f7c6293693212afdddb17de72913ecd64&head=dashpay%2Fplatform%403204fda4c3cdf5b96ad0860b9e6ea2c5d495703c)**: new nodes glow, removed ones stay as ghosts, and the tour walks through each change.

**Changed** (35 nodes)
- `tokens.distributions.once_per_identity.token.identity`
- `tokens.distributions.timed.ms.time`
- `tokens.distributions.timed.ms.time.release`
- `identities.identity`
- `contracts.contract`
- `contracts.contract.contract`
- `contracts.contract.documents`
- `contracts.contract.documents.document_type`
- `contracts.contract.documents.document_type.primary_key`
- `contracts.contract.documents.document_type.primary_key.document`
- `contracts.contract.documents.document_type.index_property`
- `contracts.contract.documents.document_type.index_property.value`
- `contracts.contract.documents.document_type.index_property.value.members`
- `contracts.contract.documents.document_type.index_property.value.members.member`
- `contracts.contract.documents.document_type.index_property.value.next_property`
- `contracts.contract.other`
- `contracts.contract.other.version`
- `contracts.contract.other.banlist.identity`
- `contracts.contract.other.suspensions.identity`
- `group_actions.contract.group.active.action.info`
- `group_actions.contract.group.active.action.signers.signer`
- `group_actions.contract.group.closed.action.info`
- `group_actions.contract.group.closed.action.signers.signer`
- `votes.contested_resource.active_polls.contract.document_type.storage.document`
- `votes.contested_resource.active_polls.contract.document_type.indexes.value`
- `votes.contested_resource.active_polls.contract.document_type.indexes.value.abstain`
- `votes.contested_resource.active_polls.contract.document_type.indexes.value.abstain.votes`
- `votes.contested_resource.active_polls.contract.document_type.indexes.value.lock`
- `votes.contested_resource.active_polls.contract.document_type.indexes.value.lock.votes`
- `votes.contested_resource.active_polls.contract.document_type.indexes.value.contender`
- and 5 more

_Compared `c47db15f7c` with `3204fda4c3`. Updated at 2026-09-20T06:51:36.337Z_