using Shop.Models;
using Shop.Services;
using Xunit;

namespace Shop.Tests;

public class UserServiceTests
{
    private readonly UserService _service = new UserService(new InMemoryRepository<User>(), new NullLogger());

    [Fact]
    public void RegistersUser()
    {
        var user = _service.Register("ada");
        Assert.Equal("ada", user.GetName());
    }

    [Theory]
    [InlineData("x")]
    public void FormatsNames(string name)
    {
        Assert.NotEmpty(Formatter.Format(name));
    }

    private sealed class NullLogger : ILogger
    {
        public void Log(string message) { }
    }
}
