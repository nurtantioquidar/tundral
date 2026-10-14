public with sharing class AccountService extends BaseService implements Queueable, Database.Batchable<SObject> {

    public interface Notifier {
        void notify(String message);
    }

    public enum Status { ACTIVE, INACTIVE }

    @AuraEnabled(cacheable=true)
    public static List<Account> getAccounts(String kind) {
        return [SELECT Id, Name FROM Account WHERE Type = :kind];
    }

    @future
    public static void archive(List<Id> ids) {
        List<Account> accounts = [SELECT Id FROM Account WHERE Id IN :ids];
        update accounts;
        delete [SELECT Id FROM Contact WHERE AccountId IN :ids];
    }

    @InvocableMethod(label='Create')
    public static void create(List<String> names) {
        insert new Account(Name = names[0]);
    }

    static void helper() {
        getAccounts('x');
    }
}
