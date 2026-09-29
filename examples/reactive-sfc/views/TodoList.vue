<script setup lang="rust">
let draft = signal(String::new());
let list: Signal<Vec<Todo>> = signal(Vec::new());
let next_id = signal(1u32);
let add = move || {
    let title = draft.get_untracked().trim().to_owned();
    if title.is_empty() {
        return;
    }
    let id = next_id.get_untracked();
    next_id.set(id + 1);
    list.update(|list| list.push(Todo { id, title }));
    draft.set(String::new());
};
</script>

<template>
  <Column :gap="8">
    <TextInput key="draft" placeholder="新任务" v-model="draft" />
    <Button key="add" :disabled="draft.with(|d| d.trim().is_empty())" @activate="add">添加</Button>
    <TodoItem
      v-for="todo in list"
      :key="todo.id"
      :todo="todo.clone()"
      @remove="list.update(|l| l.retain(|t| t.id != todo.id))"
    />
    <Text v-if="list.with(Vec::is_empty)">还没有任务</Text>
    <Text v-else>共 {{ list.with(Vec::len) }} 项</Text>
  </Column>
</template>
