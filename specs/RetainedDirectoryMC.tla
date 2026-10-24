---- MODULE RetainedDirectoryMC ----
EXTENDS OwnedDirectoryMC, RetainedDirectory

MCReceiptTarget == [receipt \in RecoveryReceiptKeys |-> 1]
MCPredecessors == [generation \in DirectoryGenerations |-> IF generation = 1 THEN {0} ELSE {}]
MCReceiptLineage == [receipt \in RecoveryReceiptKeys |-> {0, 1}]
MCGenerationRoots == [generation \in DirectoryGenerations |-> IF generation = 0 THEN {"old"} ELSE {"new"}]
MCEarlierRoot == [root \in DirectoryParents |-> IF root = "root" THEN <<"file", "prior-nested">> ELSE <<"absent">>]
=============================================================================
