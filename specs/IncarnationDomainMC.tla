---- MODULE IncarnationDomainMC ----
EXTENDS RetainedDirectoryMC, IncarnationDomain

IncMCIdentities == {<<1, 11>>, <<1, 12>>}
IncMCRoot == [identity \in IncarnationIds |-> "root"]
IncMCPredecessor == [identity \in IncarnationIds |->
    IF identity = <<1, 11>> THEN <<"symlink", "prior-root">> ELSE <<"symlink", "prior-nested">>]
IncMCReceiptKey == [identity \in IncarnationIds |-> IF identity = <<1, 11>> THEN "root-A" ELSE "root-B"]
IncMCInitialLive == [root \in DirectoryParents |-> IncarnationOwned(<<1, 11>>)]
IncMCInitialStages == {<<1, 12>>}
IncMCAncestors == [node \in DirectoryParents \union DirectoryLeaves |-> IF node = "root" THEN {} ELSE {"root"}]
IncMCBorrowedRoot == [identity \in IncarnationBorrowedIds |-> "root"]
IncMCStageName == [identity \in IncarnationIds |-> IF identity = <<1, 11>> THEN "stage-A" ELSE "stage-B"]
IncMCStageAncestors == [stage \in IncarnationStagePaths |-> {}]
IncGrowthIds == {<<1, 12>>}
IncGrowthBorrowedIds == {<<2, 12>>}
IncGrowthRoot == [identity \in IncarnationIds |-> "nested"]
IncGrowthPredecessor == [identity \in IncarnationIds |-> <<"absent">>]
IncGrowthReceiptKey == [identity \in IncarnationIds |-> "node-B"]
IncGrowthInitialLive == [root \in DirectoryParents |->
    IF root = "root" THEN IncarnationBorrowed(<<2, 12>>) ELSE <<"absent">>]
IncGrowthAncestors == [node \in DirectoryParents \union DirectoryLeaves |->
    IF node = "root" THEN {} ELSE IF node = "nested" THEN {"root"} ELSE {"root", "nested"}]
IncMCInit == RetainedDirectoryInit /\ IncarnationInit
IncMCNext == IncarnationNext /\ UNCHANGED RetainedDirectoryVars
IncMCVars == <<RetainedDirectoryVars, incarnation>>
IncMCSpec == IncMCInit /\ [][IncMCNext]_IncMCVars
=============================================================================
