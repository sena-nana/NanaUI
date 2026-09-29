<script setup lang="rust">
let draft = signal(String::new());
let list: Signal<Vec<Todo>> = signal(Vec::new());
let next_id = signal(1u32);
let draft_input = node_ref();
// Typing can start as soon as the list is shown.
on_mount(move |cx| {
    if let Some(id) = draft_input.get_untracked()
        && let Some(document) = cx.world().node(id).map(|node| node.document)
    {
        let _ = cx.focus_node(document, id);
    }
});
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
  <Column class="todos" class:empty="list.with(Vec::is_empty)" :gap="8">
    <TextInput key="draft" ref="draft_input" label="新任务" placeholder="新任务" v-model="draft" />
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

<style scoped>
/* An empty list stays in the background; the first task brings it up. */
.todos { opacity: 1; transition: opacity 120ms ease-out; }
.todos.empty { opacity: 0.6; }
</style>
