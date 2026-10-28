package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strings"
)

// nodeTask is the Task Scheduler entry `irori node install` writes.
const nodeTask = "Irori node"

// runNode is `irori node install` and `irori node uninstall`: the headless
// fallback for the always-on node (design §7, decision D3(b)). Install
// registers a Task Scheduler entry that runs `irori serve --headless` at
// logon and shows the exact command first. The recommended node is the
// desktop app minimised to the tray, which keeps the updater; a headless
// node is updated by hand. Pair it first, from the desktop app or with
// `irori pair join`.
func runNode(args []string) error {
	if len(args) != 1 || (args[0] != "install" && args[0] != "uninstall") {
		return errors.New("usage: irori node install | irori node uninstall")
	}
	if runtime.GOOS != "windows" {
		return errors.New("irori node install is for Windows (Task Scheduler); on this OS run `irori serve --headless` from your own login item, or use the desktop app")
	}
	var argv []string
	if args[0] == "install" {
		exe, err := os.Executable()
		if err != nil {
			return err
		}
		argv = nodeInstallArgs(exe)
	} else {
		argv = nodeUninstallArgs()
	}
	fmt.Println("Running:", strings.Join(quoteArgs(argv), " "))
	cmd := exec.Command(argv[0], argv[1:]...)
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("schtasks: %w", err)
	}
	if args[0] == "install" {
		fmt.Println("Irori will run headless at your next logon. Updates to a headless node are by hand.")
	}
	return nil
}

// nodeInstallArgs is the schtasks command that runs exe headless at logon,
// for this user, without raised rights. /F replaces an earlier entry, so
// running install again after moving irori.exe points it at the new path.
func nodeInstallArgs(exe string) []string {
	return []string{"schtasks", "/Create", "/TN", nodeTask, "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
		"/TR", `"` + exe + `" serve --headless`}
}

func nodeUninstallArgs() []string {
	return []string{"schtasks", "/Delete", "/TN", nodeTask, "/F"}
}

// quoteArgs quotes the arguments with spaces or quotes for display, the
// way cmd.exe would take them.
func quoteArgs(argv []string) []string {
	out := make([]string, len(argv))
	for i, a := range argv {
		if strings.ContainsAny(a, " \"") {
			a = `"` + strings.ReplaceAll(a, `"`, `\"`) + `"`
		}
		out[i] = a
	}
	return out
}
