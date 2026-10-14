namespace Shop.Core
{
    public class Item
    {
        public string Name { get; set; }
    }

    public class Card
    {
        public Item Item { get; set; }
    }

    public interface IItemService
    {
        Item Find(string name);
    }
}
