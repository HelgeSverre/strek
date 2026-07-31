-- Cursor-free AppleScript helpers for Strek's local automation endpoint.
-- Set this property to target a development binary such as target/debug/strek.
property strekBinary : missing value

on resolvedStrekBinary()
    if strekBinary is not missing value then return strekBinary
    set environmentBinary to system attribute "STREK_AUTOMATION_BINARY"
    if environmentBinary is not "" then return environmentBinary

    try
        return do shell script "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin command -v strek"
    on error
        try
            set appPath to POSIX path of (path to application "Strek")
            return appPath & "Contents/MacOS/strek"
        on error
            error "Could not find Strek. Set the strekBinary property to its executable path."
        end try
    end try
end resolvedStrekBinary

on runStrek(arguments)
    set shellCommand to quoted form of my resolvedStrekBinary() & " automate"
    repeat with argument in arguments
        set shellCommand to shellCommand & " " & quoted form of (argument as text)
    end repeat
    return do shell script shellCommand
end runStrek

on strekState()
    return my runStrek({"state"})
end strekState

on strekAction(commandId)
    return my runStrek({"action", commandId})
end strekAction

on strekPointer(phase, x, y)
    return my runStrek({"pointer", phase, x, y})
end strekPointer

on strekText(textValue)
    return my runStrek({"text", textValue})
end strekText

on strekUi(targetName, isVisible)
    if isVisible then
        set visibility to "show"
    else
        set visibility to "hide"
    end if
    return my runStrek({"ui", targetName, visibility})
end strekUi

on strekScreenshot(outputPath)
    return my runStrek({"screenshot", outputPath})
end strekScreenshot

on run arguments
    return my runStrek(arguments)
end run
