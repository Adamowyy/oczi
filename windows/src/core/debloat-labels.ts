// Readable names for the packages the debloat list offers. Brand names, so they are the same in
// every language; anything not here is prettified from its package name instead.

const LABELS: Record<string, string> = {
  "Microsoft.BingNews": "Bing News",
  "Microsoft.BingWeather": "Weather",
  "Microsoft.BingFinance": "Finance",
  "Microsoft.BingSports": "Sports",
  "Microsoft.BingFoodAndDrink": "Food & Drink",
  "Microsoft.BingTravel": "Travel",
  "Microsoft.BingHealthAndFitness": "Health & Fitness",
  "Microsoft.MicrosoftSolitaireCollection": "Solitaire Collection",
  "Microsoft.MicrosoftMahjong": "Mahjong",
  "Microsoft.MicrosoftJigsaw": "Jigsaw",
  "Microsoft.SkypeApp": "Skype",
  "SpotifyAB.SpotifyMusic": "Spotify",
  "Microsoft.ZuneVideo": "Movies & TV",
  "Microsoft.ZuneMusic": "Media Player",
  "Microsoft.WindowsCommunicationsApps": "Mail and Calendar",
  "Microsoft.People": "People",
  "Microsoft.WindowsMaps": "Maps",
  "Microsoft.MicrosoftOfficeHub": "Office hub",
  "Microsoft.Getstarted": "Tips",
  "Microsoft.GetHelp": "Get Help",
  "Microsoft.Microsoft3DViewer": "3D Viewer",
  "Microsoft.MixedReality.Portal": "Mixed Reality Portal",
  "Microsoft.WindowsFeedbackHub": "Feedback Hub",
  "Microsoft.OneConnect": "Mobile plans",
  "Microsoft.YourPhone": "Phone Link",
  "Microsoft.Todos": "Microsoft To Do",
  "Microsoft.MicrosoftStickyNotes": "Sticky Notes",
  "Microsoft.549981C3F5F10": "Cortana",
  "Clipchamp.Clipchamp": "Clipchamp",
  "Microsoft.PowerAutomateDesktop": "Power Automate",
  "Microsoft.XboxApp": "Xbox",
  "Microsoft.XboxGamingOverlay": "Xbox Game Bar",
  "Microsoft.Copilot": "Copilot",
  "MicrosoftWindows.Client.WebExperience": "Widgets",
  "Microsoft.Edge.GameAssist": "Edge Game Assist",
  "Microsoft.MicrosoftEdge.Stable": "Microsoft Edge",
  "Microsoft.MicrosoftPCManager": "PC Manager",
  "MicrosoftCorporationII.QuickAssist": "Quick Assist",
  "Microsoft.OutlookForWindows": "Outlook (new)",
  MSTeams: "Teams",
  "Microsoft.Teams": "Teams",
  "Microsoft.OneDriveSync": "OneDrive",
};

/** A readable name for a package: the brand table first, then the last dotted part with the
 *  camel-case humps split — `SomeVendor.BingNews` reads as "Bing News". */
export function debloatLabel(name: string): string {
  if (LABELS[name]) return LABELS[name];
  const tail = name.split(".").pop() ?? name;
  return tail
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .trim();
}
