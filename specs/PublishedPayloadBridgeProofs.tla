---- MODULE PublishedPayloadBridgeProofs ----
EXTENDS LifecycleRetentionProofs

CONSTANTS PayloadIdentity, PayloadModes, OldPayloadBytes, NewPayloadBytes, OldPayloadMode, NewPayloadMode,
          ObservePayload, PayloadPublicationMutant, PayloadNamespaceDepth, PayloadNamespaceMutant
VARIABLES objectCached, objectDurable, objectCachedName, objectDurableName, objectPhase,
          objectPublished, objectReturned, objectNamespaceNodes, objectNamespaceNames,
          objectNamespaceReachable, objectNamespaceCursor, objectNamespacePhase, objectNamespaceReturned
PublicationEpisode == INSTANCE ObjectNamespaceProofs
    WITH ByteValues <- Objects, ModeValues <- PayloadModes,
         OldBytes <- OldPayloadBytes, NewBytes <- NewPayloadBytes,
         OldMode <- OldPayloadMode, NewMode <- NewPayloadMode,
         Observe <- ObservePayload, Mutant <- PayloadPublicationMutant,
         NamespaceDepth <- PayloadNamespaceDepth, NamespaceMutant <- PayloadNamespaceMutant,
         cached <- objectCached, durable <- objectDurable,
         cachedName <- objectCachedName, durableName <- objectDurableName,
         phase <- objectPhase, published <- objectPublished, returned <- objectReturned,
         namespaceNodes <- objectNamespaceNodes, namespaceNames <- objectNamespaceNames,
         namespaceReachable <- objectNamespaceReachable, namespaceCursor <- objectNamespaceCursor,
         namespacePhase <- objectNamespacePhase, namespaceReturned <- objectNamespaceReturned
ASSUME PayloadPublicationDomain ==
    /\ PayloadIdentity \in Payloads
    /\ PublicationEpisode!Parameters /\ PublicationEpisode!NamespaceDomain
    /\ PayloadPublicationMutant = "none" /\ PayloadNamespaceMutant = "none"

THEOREM PublicationEpisodePremises ==
    PublicationEpisode!Parameters /\ PublicationEpisode!NamespaceDomain /\
    PublicationEpisode!CorrectProtocol /\ PublicationEpisode!CorrectNamespace
  BY SMT, PayloadPublicationDomain
  DEF PublicationEpisode!CorrectProtocol, PublicationEpisode!CorrectNamespace

\* A constructor episode ends at the client handoff. Later GC may delete an
\* unprotected object; the constructor's persistent return flag is not a claim
\* that the transferred object can never be removed. Each later publication
\* must establish this same per-call contract afresh.
AdmitPublishedPayload == PublicationEpisode!GrantObjectAuthority /\ AdmitPayload(PayloadIdentity)

THEOREM PublicationEpisodeEstablishesItsAdmissionInvariant ==
  PublicationEpisode!ObjectNamespaceSpec => []PublicationEpisode!ObjectNamespaceInvariant
  BY SMT, PublicationEpisodePremises, PublicationEpisode!CompleteObjectPublicationSafety

THEOREM PublishedPayloadHandoffProjectsLifecycle == AdmitPublishedPayload => RetentionLifecycleNext(TRUE)
  BY SMT, PayloadPublicationDomain DEF AdmitPublishedPayload, RetentionLifecycleNext

THEOREM PublishedPayloadHandoffPreservesLifecycle ==
  ASSUME RetentionLifecycleInvariant, AdmitPublishedPayload
  PROVE RetentionLifecycleInvariant'
  BY SMT, PublishedPayloadHandoffProjectsLifecycle, RetentionInduction

THEOREM PublishedPayloadHandoffHasRealDurabilityPredecessors ==
  ASSUME PublicationEpisode!ObjectNamespaceInvariant, AdmitPublishedPayload
  PROVE /\ objectCached = PublicationEpisode!NewPayload /\ objectDurable = PublicationEpisode!NewPayload
        /\ objectCachedName /\ objectDurableName
        /\ PublicationEpisode!Namespace!AllSealed /\ objectNamespaceReachable
        /\ payloadsC' = payloadsC \union {PayloadIdentity}
        /\ payloadsD' = payloadsD \union {PayloadIdentity}
  BY SMT, PublicationEpisodePremises, PublicationEpisode!ObjectReturnDischargesConsumerDurability
  DEF AdmitPublishedPayload, AdmitPayload

=============================================================================
