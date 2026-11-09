// useMe is you as your profile has you (card #393): your name (#183),
// colour (#385) and avatar, for the sidebar's row and your own faces. It
// reads them again when the profile page changes one (meChanged) and when
// the Core says a device or person pick changed (another of your devices
// picked).

import { useEffect } from "react"

import { devices } from "./devices"
import { Events, SettingsService } from "./host"
import { EventDevicesChanged, inWails } from "./irori"
import { useLatest } from "./talk"

const MeEvent = "irori:me"

// meChanged tells every useMe to read again.
export function meChanged(): void {
  window.dispatchEvent(new Event(MeEvent))
}

export type Me = { name: string; color: string; avatar: string }

const sampleMe: Me = { name: "Sample Person", color: "", avatar: "" }

async function readMe(): Promise<Me> {
  const [name, color, avatar] = await Promise.all([
    inWails() ? SettingsService.GetYourName() : Promise.resolve(sampleMe.name),
    devices.personColor(),
    devices.personAvatar(),
  ])
  return { name: name ?? "", color, avatar }
}

export function useMe(): Me {
  const { value, reload } = useLatest<Me>(
    readMe,
    inWails() ? { name: "", color: "", avatar: "" } : sampleMe,
    "me"
  )
  useEffect(() => {
    void reload()
    const on = () => void reload()
    window.addEventListener(MeEvent, on)
    const off = inWails() ? Events.On(EventDevicesChanged, on) : () => {}
    return () => {
      window.removeEventListener(MeEvent, on)
      off()
    }
  }, [reload])
  return value
}
