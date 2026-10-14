trigger AccountTrigger on Account (before insert, after update) {
    if (Trigger.isBefore) {
        AccountService.helper();
    }
    upsert Trigger.new;
}
