// The existing PC installer requires an executable version and permission probe.
// Replacing the desktop with this marker lets its normal cleanup delete the
// entire capture runtime, including libraries and platform helpers.
package main

import "os"

func main() {
	if len(os.Args) == 2 {
		switch os.Args[1] {
		case "--version":
			os.Stdout.WriteString("Lazycat Remote 0.0.0-uninstalled\n")
			return
		case "permissions":
			os.Stdout.WriteString("{\"screen\":\"denied\",\"input\":\"denied\",\"can_start\":false}\n")
			return
		}
	}
	os.Exit(1)
}
