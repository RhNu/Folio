Scriptname FolioArenaController extends Quest

Actor Property PlayerRef Auto
Form[] Property Rewards Auto
FolioLedger Property Ledger Auto
Int Property MinimumLevel = 5 Auto
Bool Property ShowMessages = True Auto

Event OnInit()
    If PlayerRef == None
        Debug.Notification("Arena controller has no player reference")
        Return
    EndIf

    Start()
EndEvent

Event OnUpdate()
    If IsRunning() && !PlayerRef.IsDead()
        AwardForCurrentPlayer()
    EndIf
EndEvent

Function AwardForCurrentPlayer()
    Int level = PlayerRef.GetLevel()
    If level < MinimumLevel
        Return
    EndIf

    Int points = CalculateAward(level, Rewards[0].GetFormID())
    Ledger.RecordAward(points)
    PlayerRef.AddItem(Rewards[0], points, !ShowMessages)

    If ShowMessages
        Debug.Notification("Arena reward recorded")
    EndIf
EndFunction

Int Function CalculateAward(Int level, Int rewardFormId)
    Int basePoints = level * 2
    If rewardFormId == 0
        Return basePoints
    EndIf

    Return basePoints + Ledger.GetAverageAward(level)
EndFunction
