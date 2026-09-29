<script setup lang="rust">
defineProps!(list: Signal<Vec<Item>>);
</script>

<template>
  <Column :gap="4">
    <ListRow
      v-for="item in list"
      :key="item.id"
      :title="item.title"
      @remove="list.update(|l| l.retain(|t| t.id != item.id))"
    />
    <Text v-if="list.with(Vec::is_empty)">还没有任务</Text>
    <Text v-else>共 {{ list.with(Vec::len) }} 项</Text>
  </Column>
</template>
