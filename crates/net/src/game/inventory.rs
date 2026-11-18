//! The player inventory the game thread owns (Шаг 24a).
//!
//! The vanilla player inventory is a 46-slot menu (`0` crafting result,
//! `1..=4` the 2×2 grid, `5..=8` armor, `9..=35` main, `36..=44` hotbar,
//! `45` offhand). Шаг 24a stores every slot, because one packet
//! (`set_creative_mode_slot`) can name any of them, while the save writes only
//! the hotbar (Шаг 24d).
//!
//! SOURCE: the S24a/S17 vanilla traces (`docs/BLOCK-PLACING.md` §2, §5).
//! The join `container_set_content` carries 46 slots and window state id `1`;
//! a creative pick puts the item into the first empty hotbar slot from `36`,
//! sends `set_held_slot` first and then `container_set_slot` with the next
//! state id. Slot numbering is the vanilla menu numbering (the same values the
//! traces carry).
//!
//! Component bytes of a stack are kept verbatim by [`protocol::ItemStack`];
//! this module never inspects them. A stack with components is not written to
//! the player file (Шаг 24d debt: the NBT form of a component is not modelled).

use protocol::ItemStack;
use tracing::debug;

use data::RegistryIds;
use playerdata::SavedItem;

/// Slots of the player-inventory menu.
pub const SLOT_COUNT: usize = 46;
/// First slot of the hotbar (`36`).
pub const HOTBAR_START: i16 = 36;
/// Last slot of the hotbar (`44`).
pub const HOTBAR_END: i16 = 44;
/// First slot of the main inventory (`9`).
pub const MAIN_START: i16 = 9;
/// Last slot of the main inventory (`35`).
pub const MAIN_END: i16 = 35;
/// Number of hotbar slots (`9`).
pub const HOTBAR_SLOTS: i16 = 9;
/// The registry the creative item ids belong to.
pub const ITEM_REGISTRY: &str = "minecraft:item";

/// What a creative pick changed, in the order the answer must be sent.
#[derive(Debug, Clone, PartialEq)]
pub struct PickResult {
    /// Selected hotbar slot (`0..=8`); the answer starts with
    /// `set_held_slot` of this value.
    pub held: i32,
    /// New slot content to announce after `set_held_slot`, if the item had to
    /// be added: `(menu slot, state id)`.
    pub added: Option<(i16, i32)>,
}

/// The 46 menu slots, the selected hotbar slot and the window state counter.
#[derive(Debug, Clone)]
pub struct Inventory {
    slots: Vec<ItemStack>,
    selected: i32,
    state_id: i32,
}

impl Default for Inventory {
    fn default() -> Self {
        Self::empty()
    }
}

impl Inventory {
    /// An empty inventory: all 46 slots air, slot 0 selected and the window
    /// state id `1` (the value the join `container_set_content` carries).
    pub fn empty() -> Self {
        Self {
            slots: vec![ItemStack::empty(); SLOT_COUNT],
            selected: 0,
            state_id: 1,
        }
    }

    /// Builds the inventory from the hotbar of a player file (Шаг 24d).
    ///
    /// `Slot` of the file is the player-file slot space: `0..=8` are the
    /// hotbar menu slots `36..=44`. An item the item registry does not know
    /// is skipped with a debug line instead of failing the join.
    pub fn from_saved(saved: &[SavedItem], items: Option<&RegistryIds>) -> Self {
        let mut inventory = Self::empty();
        let Some(items) = items else {
            if !saved.is_empty() {
                debug!("no item registry: the saved hotbar is not restored");
            }
            return inventory;
        };
        for item in saved {
            if !(0..i32::from(HOTBAR_SLOTS)).contains(&item.slot) {
                continue;
            }
            let Some(id) = items.entry_id(ITEM_REGISTRY, &item.id) else {
                debug!(
                    item = %item.id,
                    "the saved hotbar item is not in the item registry, skipped"
                );
                continue;
            };
            let slot = usize::try_from(HOTBAR_START + item.slot as i16).unwrap_or(0);
            if let Some(target) = inventory.slots.get_mut(slot) {
                *target = ItemStack::new(id, item.count);
            }
        }
        inventory
    }

    /// Every menu slot, in menu order.
    pub fn slots(&self) -> &[ItemStack] {
        &self.slots
    }

    /// The selected hotbar slot (`0..=8`).
    pub fn selected(&self) -> i32 {
        self.selected
    }

    /// The window state id the next `container_set_slot` must carry.
    pub fn state_id(&self) -> i32 {
        self.state_id
    }

    /// The stack in the selected hotbar slot.
    pub fn held(&self) -> &ItemStack {
        let index = usize::try_from(HOTBAR_START + self.selected as i16).unwrap_or(0);
        self.slots.get(index).unwrap_or(&self.slots[0])
    }

    /// The stack in one menu slot, or `None` outside `0..=45`.
    pub fn stack(&self, slot: i16) -> Option<&ItemStack> {
        if !(0..SLOT_COUNT as i16).contains(&slot) {
            return None;
        }
        self.slots.get(usize::from(slot as u16))
    }

    /// Selects a hotbar slot; `false` (and no change) outside `0..=8`, the
    /// way the S24a trace's ids are always valid.
    pub fn select(&mut self, slot: i16) -> bool {
        if !(0..HOTBAR_SLOTS).contains(&slot) {
            return false;
        }
        self.selected = i32::from(slot);
        true
    }

    /// Writes one creative slot (`set_creative_mode_slot`); `false` (and no
    /// change) for a menu slot outside `0..=45`.
    ///
    /// The trace has no clientbound answer to this packet, so the write is
    /// silent even when it clears a slot.
    pub fn set_creative(&mut self, slot: i16, stack: ItemStack) -> bool {
        if !(0..SLOT_COUNT as i16).contains(&slot) {
            return false;
        }
        if let Some(target) = self.slots.get_mut(usize::from(slot as u16)) {
            *target = stack;
            return true;
        }
        false
    }

    /// The creative pick of one item (S17): select a hotbar slot that already
    /// holds it, otherwise add it to the first empty hotbar slot.
    ///
    /// The trace rule: an existing match sends only `set_held_slot`; a new
    /// item is put into the first empty hotbar slot (`36` first) and the
    /// answer is `set_held_slot` and then `container_set_slot`. When the hotbar
    /// is full the item goes to the first empty main slot; when the whole
    /// inventory is full the selected slot is overwritten.
    // TODO(24a): the full-hotbar case is not in any trace. Vanilla's exact
    // choice (main slot without selecting it, or the selected slot) is
    // unknown; the S17 trace only shows an empty hotbar being filled.
    pub fn pick(&mut self, stack: ItemStack) -> PickResult {
        if let Some(hotbar) = self.find_hotbar_match(&stack) {
            self.selected = i32::from(hotbar - HOTBAR_START);
            return PickResult {
                held: self.selected,
                added: None,
            };
        }
        let slot = self
            .first_empty(HOTBAR_START, HOTBAR_END)
            .or_else(|| self.first_empty(MAIN_START, MAIN_END))
            .unwrap_or(HOTBAR_START + self.selected as i16);
        let index = usize::from(slot as u16);
        self.slots[index] = stack;
        if (HOTBAR_START..=HOTBAR_END).contains(&slot) {
            self.selected = i32::from(slot - HOTBAR_START);
        }
        self.state_id = self.state_id.wrapping_add(1);
        PickResult {
            held: self.selected,
            added: Some((slot, self.state_id)),
        }
    }

    /// Hotbar index (`0..=8`) of a slot holding the same item as `stack`.
    ///
    /// Vanilla matches the item and its components; the traces only carry
    /// component-free stacks, so the component bytes must be equal too.
    fn find_hotbar_match(&self, stack: &ItemStack) -> Option<i16> {
        (HOTBAR_START..=HOTBAR_END).find(|slot| {
            let current = &self.slots[usize::from(*slot as u16)];
            !current.is_empty()
                && current.item == stack.item
                && current.components == stack.components
        })
    }

    /// First empty menu slot in `start..=end`, if any.
    fn first_empty(&self, start: i16, end: i16) -> Option<i16> {
        (start..=end).find(|slot| self.slots[usize::from(*slot as u16)].is_empty())
    }

    /// The non-empty hotbar stacks, in the player-file slot space
    /// (`0..=8`), for the save (Шаг 24d).
    ///
    /// A stack the item registry cannot name, or one that carries components,
    /// is skipped: the file stores an identifier and the NBT component form is
    /// not modelled. Skipping is logged, never silent.
    pub fn saved_hotbar(&self, items: Option<&RegistryIds>) -> Vec<SavedItem> {
        let mut saved = Vec::new();
        let Some(items) = items else {
            return saved;
        };
        for slot in 0..HOTBAR_SLOTS {
            let stack = &self.slots[usize::from((HOTBAR_START + slot) as u16)];
            if stack.is_empty() {
                continue;
            }
            if !stack.components.is_empty() && stack.components != [0, 0] {
                debug!(
                    slot,
                    "a hotbar stack with components is not saved (Шаг 24d debt)"
                );
                continue;
            }
            let Some(name) = items.entry_name(ITEM_REGISTRY, stack.item) else {
                debug!(
                    item = stack.item,
                    slot, "a hotbar item has no registry name, not saved"
                );
                continue;
            };
            saved.push(SavedItem {
                slot: i32::from(slot),
                count: stack.count,
                id: name.to_owned(),
            });
        }
        saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The S17 report slice with the ids the trace carries.
    fn registries() -> RegistryIds {
        data::registries_from_json(
            r#"{"minecraft:item": {"entries": {
                "minecraft:stone": {"protocol_id": 1},
                "minecraft:dirt": {"protocol_id": 55},
                "minecraft:oak_planks": {"protocol_id": 63},
                "minecraft:glass": {"protocol_id": 231},
                "minecraft:oak_stairs": {"protocol_id": 515}
            }}}"#,
        )
        .expect("valid registries report")
    }

    /// The hotbar menu slots of an inventory, top to bottom.
    fn hotbar(inventory: &Inventory) -> Vec<&ItemStack> {
        (HOTBAR_START..=HOTBAR_END)
            .map(|slot| &inventory.slots()[usize::from(slot as u16)])
            .collect()
    }

    #[test]
    fn a_new_inventory_is_46_empty_slots_with_state_one() {
        let inventory = Inventory::empty();
        assert_eq!(inventory.slots().len(), SLOT_COUNT);
        assert!(inventory.slots().iter().all(ItemStack::is_empty));
        assert_eq!(inventory.selected(), 0);
        assert_eq!(inventory.state_id(), 1);
        assert!(inventory.held().is_empty());
    }

    #[test]
    fn a_creative_slot_is_written_and_cleared_silently() {
        let mut inventory = Inventory::empty();
        assert!(inventory.set_creative(36, ItemStack::new(1, 64)));
        assert_eq!(inventory.slots()[36].item, 1);
        assert_eq!(inventory.slots()[36].count, 64);
        // The state id is only moved by a reply that announces a change;
        // set_creative_mode_slot has no clientbound answer.
        assert_eq!(inventory.state_id(), 1);

        assert!(inventory.set_creative(36, ItemStack::empty()));
        assert!(inventory.slots()[36].is_empty());
        assert!(!inventory.set_creative(46, ItemStack::new(1, 1)));
        assert!(!inventory.set_creative(-1, ItemStack::new(1, 1)));
    }

    #[test]
    fn selecting_a_hotbar_slot_rejects_anything_outside_zero_to_eight() {
        let mut inventory = Inventory::empty();
        assert!(inventory.select(8));
        assert_eq!(inventory.selected(), 8);
        assert!(!inventory.select(9));
        assert!(!inventory.select(-1));
        assert_eq!(inventory.selected(), 8);
    }

    /// The S17 pick rule: a new item lands in the first empty hotbar slot and
    /// moves the state id; the same item again only selects its slot.
    #[test]
    fn a_pick_adds_to_the_first_empty_hotbar_slot_and_then_matches_it() {
        let mut inventory = Inventory::empty();
        let first = inventory.pick(ItemStack::new(1, 1));
        assert_eq!(
            first,
            PickResult {
                held: 0,
                added: Some((36, 2))
            }
        );
        assert_eq!(inventory.slots()[36].item, 1);

        let second = inventory.pick(ItemStack::new(231, 1));
        assert_eq!(
            second,
            PickResult {
                held: 1,
                added: Some((37, 3))
            }
        );
        assert_eq!(inventory.slots()[37].item, 231);

        // Stone is already in slot 36: only `set_held_slot` is sent, and the
        // state id does not move.
        let again = inventory.pick(ItemStack::new(1, 1));
        assert_eq!(
            again,
            PickResult {
                held: 0,
                added: None
            }
        );
        assert_eq!(inventory.state_id(), 3);
    }

    #[test]
    fn a_full_hotbar_falls_back_to_the_main_inventory() {
        let mut inventory = Inventory::empty();
        for slot in HOTBAR_START..=HOTBAR_END {
            inventory.set_creative(slot, ItemStack::new(1, 64));
        }
        let picked = inventory.pick(ItemStack::new(231, 1));
        assert_eq!(
            picked,
            PickResult {
                held: 0,
                added: Some((MAIN_START, 2))
            }
        );
        assert_eq!(inventory.slots()[MAIN_START as usize].item, 231);
    }

    #[test]
    fn the_saved_hotbar_maps_names_to_ids_and_back() {
        let items = registries();
        let inventory = Inventory::from_saved(
            &[
                SavedItem {
                    slot: 0,
                    count: 64,
                    id: "minecraft:stone".to_owned(),
                },
                SavedItem {
                    slot: 4,
                    count: 1,
                    id: "minecraft:oak_stairs".to_owned(),
                },
                SavedItem {
                    slot: 2,
                    count: 1,
                    id: "minecraft:not_an_item".to_owned(),
                },
            ],
            Some(&items),
        );
        assert_eq!(inventory.slots()[36].item, 1);
        assert_eq!(inventory.slots()[40].item, 515);
        assert!(
            inventory.slots()[38].is_empty(),
            "the unknown id is skipped"
        );

        let saved = inventory.saved_hotbar(Some(&items));
        assert_eq!(
            saved,
            vec![
                SavedItem {
                    slot: 0,
                    count: 64,
                    id: "minecraft:stone".to_owned()
                },
                SavedItem {
                    slot: 4,
                    count: 1,
                    id: "minecraft:oak_stairs".to_owned()
                },
            ]
        );
    }

    #[test]
    fn a_hotbar_stack_with_components_is_not_saved() {
        let items = registries();
        let mut inventory = Inventory::empty();
        let mut stack = ItemStack::new(1, 1);
        stack.components = vec![1, 0x2a, 0x05, 0];
        inventory.set_creative(36, stack);
        assert!(inventory.saved_hotbar(Some(&items)).is_empty());
        assert_eq!(hotbar(&inventory).len(), 9);
    }
}
