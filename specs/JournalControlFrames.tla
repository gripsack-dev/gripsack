---- MODULE JournalControlFrames ----
EXTENDS JournalEpochProofs

THEOREM ProcessDeathSetsRecoveryStage ==
  ASSUME NEW c \in CellSpace
  PROVE ProcessDeathImage(c).control.stage = "recover"
  BY SMT DEF ProcessDeathImage, CellSpace, ControlSpace

THEOREM PowerLossSetsRecoveryStage ==
  ASSUME NEW c \in CellSpace, NEW next \in PowerLossImages(c)
  PROVE next.control.stage = "recover"
  BY SMT DEF PowerLossImages, PowerLossImage, CellSpace, DiskSpace, ControlSpace

THEOREM RecoveryImagesKeepStage ==
  ASSUME NEW c \in CellSpace, c.control.stage = "recover",
         NEW next \in {RestorationImage(c), PriorSealImage(c), KeepImage(c), EntryRemovalImage(c)}
  PROVE next.control.stage = "recover"
  BY SMT DEF RestorationImage, PriorSealImage, KeepImage, EntryRemovalImage,
             CellSpace, DiskSpace, ControlSpace

THEOREM WritebackKeepsStage ==
  ASSUME NEW c \in CellSpace,
         NEW next \in {EntryWritebackImage(c), LiveWritebackImage(c)}
  PROVE next.control.stage = c.control.stage
  BY SMT DEF EntryWritebackImage, LiveWritebackImage, CellSpace, DiskSpace

THEOREM ExternalEditKeepsEntryControl ==
  ASSUME NEW c \in CellSpace, NEW value \in Objects
  PROVE /\ ExternalEditImage(c, value).cached.entry = c.cached.entry
        /\ ExternalEditImage(c, value).durable.entry = c.durable.entry
        /\ ExternalEditImage(c, value).control = c.control
  BY SMT DEF ExternalEditImage, CellSpace, DiskSpace

=============================================================================
