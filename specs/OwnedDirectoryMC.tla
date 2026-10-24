---- MODULE OwnedDirectoryMC ----
EXTENDS OwnedDirectory

\* TLC instantiates the same arbitrary-finite relation used by the induction.
\* Numeric tuple labels are opaque qualified-birth-identity tokens, NOT a
\* concrete device/inode encoding; no model action compares a token component.
MCIdentities == {<<1, 11>>, <<1, 12>>, <<2, 11>>, <<2, 12>>}
MCPrepared == [parent \in DirectoryParents |-> IF parent = "root" THEN <<1, 11>> ELSE <<1, 12>>]
MCForeign == [parent \in DirectoryParents |-> IF parent = "root" THEN <<2, 11>> ELSE <<2, 12>>]
MCOriginal == [parent \in DirectoryParents |-> IF parent = "root" THEN "prior-root" ELSE "prior-nested"]
MCAncestors == [node \in DirectoryParents \union DirectoryLeaves |->
    IF node = "root" THEN {} ELSE IF node = "nested" THEN {"root"} ELSE {"root", "nested"}]
=============================================================================
