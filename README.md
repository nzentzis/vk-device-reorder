# Summary
This is a Vulkan implicit layer capable of re-ordering the list that's returned
when applications call `vkEnumeratePhysicalDevices`. You can set per-device
priority rules, or hide devices from the returned list entirely, under the
control of either a global or process-local (controlled by environment vars)
configuration file.

# How is this useful?
When Vulkan applications initialize, they need to choose a physical device to
use for rendering, and they query the Vulkan runtime using the
`vkEnumeratePhysicalDevices` function to retrieve a list they can choose from.
The Vulkan standard allows this list to be in arbitrary order, and it's up to
the developer to choose the right device from the list. Most applications I've
seen just take the first device marked as a discrete GPU and use that.

However, this behavior can cause issues on multi-GPU systems where only one of
the discrete cards available is connected to the monitor, and the rest are used
for compute only. Vulkan doesn't provide a way to distinguish devices that can
be used for *rendering* from those that are only useful for *compute*. As a
result, Vulkan applications on those systems may fail to create rendering
contexts and crash as a result.

This layer allows the user to resolve this problem non-invasively, without
modifying the Vulkan applications themselves, by reordering the device list
returned by the driver so that applications' naive "pick first GPU" behavior
will select the correct rendering device.

# Caveats
Applications may use different selection policies (though I haven't found any
that do), in which case simply reordering the list of devices may not be enough.
In this case, you can still use the `hide` option to hide other GPUs from the
application, but for compute workloads this may prevent the software from fully
utilizing your available cards.

This tool may cause issues if used on a game with aggressive anti-cheat. In this
case, you can use the `DISABLE_DEVICE_REORDER` environment variable defined in
the manifest to skip loading the layer, though depending on the game this may
cause it to fail to find your GPU.

I've only tested this on Linux, and I make no guarantees about its functionality
on Windows platforms. If you want to make a PR improving Windows support, it'd
be appreciated, but I'm unable to test or maintain such support as I don't use
Windows.

# Installing
To use this, simply clone the repo and build the library. You'll need a Rust
compiler and toolchain installed, which you can get via
[Rustup](https://rustup.rs).

```
$ git clone ...
$ cd vk-device-reorder
$ cargo build --release
```

Once the library is built, just copy it into a convenient location and install
the layer manifest:

```
$ sudo cp target/release/libvk_device_reorder.so /usr/local/lib/
$ sudo cp manifest.json /usr/share/vulkan/implicit_layer.d/vk_device_reorder.json
$ sudo chown root:root /usr/local/lib/libvk_device_reorder.so
$ sudo chown root:root /usr/share/vulkan/implicit_layer.d/vk_device_reorder.json
```

Then just drop a config file into one of the configuration paths described
below. To apply globally, you'd put the file in `/etc/vk_device_reorder.json`
(and change the owner to root, since it's in `/etc`).

# Configuring
When loaded, the layer will attempt to read a JSON-formatted configuration file,
looking at the following paths in order:

 * The path in the `VK_REORDER_CONFIG` environment var, if set.
 * `vk_device_reorder.json` in the current directory.
 * `/etc/vk_device_reorder.json`

If none of those can be loaded, the layer uses the default behavior, which
contains no rules and thereofre treats all devices with priority 0. A malformed
config file is equivalent to a missing config file, and will be ignored.

Configuration files are structured as follows:

```json
{
    "rules": [
        {
            "card_name": "NVIDIA GeForce GTX 1080 Ti"
            "priority": 5,
        },
        {
            "card_name": "Some other card",
            "hide": true
        }
    ]
}
```

The root object contains a `rules` key, which is a list of rule objects. Each
device in the device list is matched against all rules. Devices with one or more
rules that set the `hide` property will be removed from the list visible to the
application. Non-hidden devices will be assigned a priority equal to the sum of
the priorities set on all matching rules, then sorted in reverse priority order.
Rules without a specified priority will be considered to have a priority of
zero.

In other words, to move a device closer to the front of the list, increase its
priority by adding a rule matching it with a positive priority. To move a device
further down the device list, use a negative priority.

Rules can use the following keys to match devices. All keys must match a given
device for the rule to apply, and missing keys are considered to match by
default.

 * `card_name` - Match the name of the device, as returned by the Vulkan runtime
 * `is_display` - If set, match whether the card is known to be a display. Note
   that this value is currently unreliable, so use of this key is discouraged.

Additionally, if the `invert` key is set to `true` for a rule, its matching
behavior will be reversed. This allows you to create rules which down-score the
cards you don't want, if it's easier to select those.

# Contributions and Bug Reports
Contributions and bug reports are welcome, but please note that I may not always
be able to replicate issues that relate to specific applications, drivers, or OS
versions, so you might need to help test changes to get them fixed.
