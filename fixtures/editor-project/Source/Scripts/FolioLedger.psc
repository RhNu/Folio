Scriptname FolioLedger

Int Property TotalAwards Auto Hidden
Int Property LastAward Auto Hidden

Function RecordAward(Int points)
    TotalAwards = TotalAwards + points
    LastAward = points
EndFunction

Int Function GetAverageAward(Int sessions)
    If sessions <= 0
        Return 0
    EndIf

    Return TotalAwards / sessions
EndFunction
