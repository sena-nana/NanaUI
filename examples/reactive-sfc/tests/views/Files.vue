<script setup lang="rust">
use nana_ui::runtime::Locale;

defineProps!(owner: String);
let count = signal(1u64);
// Nothing writes `seven`: the message that reads it is written once.
let seven = signal(7u64);
let chosen: Signal<Option<Locale>> = signal(None);
</script>

<template>
  <Column :gap="4">
    <T key="title" id="title" />
    <T key="count" id="files" :count="count" />
    <T key="owned" id="owned" :count="seven" :owner="owner" />
    <Column key="arabic" locale="ar">
      <T key="text" id="title" />
    </Column>
    <Column key="chosen" :locale="chosen">
      <T key="text" id="files" :count="count" />
    </Column>
    <Button key="more" @activate="count.update(|c| *c += 1)">+</Button>
    <Button key="chinese" @activate="chosen.set(Locale::parse(&quot;zh-cn&quot;))">中文</Button>
  </Column>
</template>
